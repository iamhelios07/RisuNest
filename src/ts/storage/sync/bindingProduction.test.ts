import { expect, it, vi } from 'vitest'
vi.mock('./bindingDialog', () => ({ confirmSyncBindingReplacement: vi.fn(async () => true) }))
vi.mock('./bindingLocalData', () => ({ hasLocalBindingData: vi.fn(async () => true), hasLocalSharedBindingData: vi.fn(async () => true) }))
import { IDBFactory, IDBKeyRange } from 'fake-indexeddb'
import { IndexedDbPersistentDataStore } from '../indexedDbPersistentDataStore'
import { createPersistentDataRuntime, capturePersistentRoot, capturePersistentPresets, capturePersistentPluginStorage } from '../persistentDataRuntime'
import type { PersistentMutationToken } from '../saveCoordinator'
import { makeDatabase } from '../saveCoordinator.testSupport'
import { createStorageMutationGate, createInRealmStorageLockManager } from '../storageMutationGate'
import { createSyncBindingFlow, type SyncBindingDependencies, type SyncBindingState, type SyncBindingTransport } from './bindingFlow'
import { createSyncBindingRecoveryRegistration } from './bindingProduction'

async function harness() {
    let database = makeDatabase()
    const events: string[] = []
    const cursor = vi.fn(async (revision: number) => { events.push(`cursor-${revision}`) })
    const store = Object.assign(new IndexedDbPersistentDataStore('binding-recovery', new IDBFactory(), IDBKeyRange), {
        commitWorkingSetChangeCursor: cursor,
    })
    await store.open()
    await store.replaceFromDatabase(database, (await store.readRoot()).revision)
    const runtime = createPersistentDataRuntime({
        store,
        state: {
            captureRoot: () => capturePersistentRoot(database),
            capturePresets: () => capturePersistentPresets(database),
            capturePluginStorage: () => capturePersistentPluginStorage(database),
            captureSelectedCharacter: () => database.characters[0] ?? null,
            captureCharacter: id => database.characters.find(character => character.chaId === id) ?? null,
            getSelectedCharacterId: () => database.characters[0]?.chaId,
            getSelectedConversationId: () => null,
            replaceDatabase: replacement => { database = replacement; events.push('projection') },
            publishCharacter: () => {}, publishConversation: () => {}, isConversationOperationActive: () => false,
        },
        prepareDatabase: async value => structuredClone(value),
        clock: { setTimeout: () => 1, clearTimeout: () => {} },
    })
    await runtime.initializeActiveWorkingSet(database)
    events.length = 0
    let token: PersistentMutationToken | undefined
    const pausedToken = () => { if (!token) throw Error('pause missing'); return token }
    let state: SyncBindingState = { target: { kind: 'none' }, targetAuthority: '0', selectionEpoch: 'old', libraryId: null, progress: null }
    const target = { kind: 'server', connectionId: 'synthetic' } as const
    let switchReceipt: { body: string; state: SyncBindingState } | undefined
    const dependencies: SyncBindingDependencies = {
        native: {
            state: async () => structuredClone(state),
            assertAuthority: async expected => {
                if (expected.targetAuthority !== state.targetAuthority || expected.selectionEpoch !== state.selectionEpoch) throw Error('stale binding')
            },
            switchTarget: async (previous, selected, inspection, requestId) => {
                const body = JSON.stringify({ previous, selected, inspection, requestId })
                if (switchReceipt) {
                    if (body !== switchReceipt.body || state.selectionEpoch !== switchReceipt.state.selectionEpoch) throw Error('replay changed')
                    return structuredClone(switchReceipt.state)
                }
                expect(previous.targetAuthority).toBe('0')
                state = { target: selected, targetAuthority: '1', selectionEpoch: 'new', libraryId: inspection!.libraryId, progress: null }
                switchReceipt = { body, state: structuredClone(state) }
                events.push('switch')
                return structuredClone(state)
            },
        },
        gate: createStorageMutationGate({ locks: createInRealmStorageLockManager() }),
        withPausedWrites: operation => runtime.withPausedPersistentWrites('binding-test', async paused => {
            token = paused
            try { return await operation() } finally { token = undefined; events.push('pause-release') }
        }),
        beginActivatedLibraryGuard: () => runtime.beginActivatedLibraryGuard(pausedToken()),
        refreshActivatedLibrary: async () => { expect((await runtime.refreshActivatedLibraryUnderPause(pausedToken())).projection).toBe('applied') },
        recovery: createSyncBindingRecoveryRegistration(() => runtime, pausedToken),
        hasNonDefaultData: async () => true, hasNonDefaultSharedData: async () => true, confirmReplacement: async () => true,
        plugins: { fenceExecution: async () => {}, invalidateCaches: async () => {}, restart: async () => { events.push('restart') } },
    }
    let replacementRevision: number | undefined
    const transport: SyncBindingTransport = {
        inspectTarget: async () => ({ inspectionId: 'inspection', targetId: 'target', libraryId: 'library', empty: false, previouslyBoundLibrary: false }),
        pullAvailableState: vi.fn(async () => ({ targetId: 'target', libraryId: 'library', stagingId: 'stage', receiveId: 'receive' })),
        replaceFromTarget: async () => {
            if (replacementRevision === undefined) {
                replacementRevision = (await store.replaceFromDatabase({ ...makeDatabase(), username: 'Activated synthetic target' }, (await store.readRoot()).revision)).revision
                events.push('activate')
            }
        },
        fenceOldJobs: async () => {}, publishInitialSharedState: vi.fn(async () => {}),
        resumeBinding: vi.fn(async () => { events.push('resume') }),
    }
    return { runtime, store, dependencies, transport, target, events, cursor, flow: createSyncBindingFlow(dependencies),
        get database() { return database }, setState: (next: SyncBindingState) => { state = next },
        changeAuthority: () => { state.targetAuthority = 'unrelated'; state.selectionEpoch = 'unrelated' } }
}

it('keeps real runtime writes fenced after partial restart, then ordinary retry restarts and resumes once', async () => {
    const h = await harness()
    let restarts = 0
    h.dependencies.plugins.restart = async () => {
        expect(h.database.username).toBe('Activated synthetic target')
        expect(() => h.runtime.markPersistentDataDirty(1)).toThrow()
        await expect(h.runtime.commitPersistentUnitIntent('plugin-init', [{ type: 'set', key: JSON.stringify(['root', 'username']), value: 'Rejected init' }])).rejects.toThrow()
        h.events.push('restart')
        if (++restarts === 1) throw Error('partial restart failed')
    }
    await expect(h.flow.bind(h.target, h.transport)).rejects.toThrow('partial restart failed')
    expect(() => h.runtime.markPersistentDataDirty(1)).toThrow()
    expect(h.transport.resumeBinding).not.toHaveBeenCalled()
    expect((await h.runtime.retryCommittedWorkingSetRefresh())?.projection).toBe('applied')
    expect(() => h.runtime.markPersistentDataDirty(1)).not.toThrow()
    expect(restarts).toBe(2)
    expect(h.transport.resumeBinding).toHaveBeenCalledOnce()
    expect(h.events.lastIndexOf('restart')).toBeLessThan(h.events.indexOf('resume'))
    expect(h.events.filter(event => event === 'activate')).toHaveLength(1)
    await expect(h.runtime.retryCommittedWorkingSetRefresh()).resolves.toBeNull()
})

it('retains a failed post-release adapter resume for ordinary runtime retry', async () => {
    const h = await harness()
    const resume = vi.fn(async () => {
        expect(() => h.runtime.markPersistentDataDirty(1)).not.toThrow()
        if (resume.mock.calls.length === 1) throw Error('adapter resume failed')
        h.events.push('resume')
    })
    h.transport.resumeBinding = resume
    await expect(h.flow.bind(h.target, h.transport)).rejects.toThrow('adapter resume failed')
    expect(() => h.runtime.markPersistentDataDirty(1)).toThrow()
    expect((await h.runtime.retryCommittedWorkingSetRefresh())?.projection).toBe('applied')
    expect(resume).toHaveBeenCalledTimes(2)
    expect(h.events.filter(event => event === 'resume')).toHaveLength(1)
    await expect(h.runtime.retryCommittedWorkingSetRefresh()).resolves.toBeNull()
})

it.each([false, true])('retries initialized adapter resume without repeating initial publication (shared data: %s)', async nonDefault => {
    const h = await harness()
    h.transport.inspectTarget = async () => ({ inspectionId: 'inspection', targetId: 'target', libraryId: 'library', empty: true, previouslyBoundLibrary: false })
    h.dependencies.hasNonDefaultSharedData = async () => nonDefault
    const resume = vi.fn(async () => { if (resume.mock.calls.length === 1) throw Error('adapter resume failed') })
    h.transport.resumeBinding = resume
    await expect(h.flow.bind(h.target, h.transport)).rejects.toThrow('adapter resume failed')
    expect(h.transport.publishInitialSharedState).toHaveBeenCalledTimes(nonDefault ? 1 : 0)
    expect((await h.runtime.retryCommittedWorkingSetRefresh())?.projection).toBe('applied')
    expect(resume).toHaveBeenCalledTimes(2)
    expect(h.transport.publishInitialSharedState).toHaveBeenCalledTimes(nonDefault ? 1 : 0)
    expect(h.events).not.toContain('activate')
})

it.each(['switch', 'replacement'])('settles a lost %s response by exact replay before recovery projection', async phase => {
    const h = await harness()
    const original = phase === 'switch' ? h.dependencies.native.switchTarget : h.transport.replaceFromTarget
    let calls = 0
    const bodies: string[] = []
    if (phase === 'switch') h.dependencies.native.switchTarget = async (...args) => {
        bodies.push(JSON.stringify(args))
        const result = await (original as typeof h.dependencies.native.switchTarget)(...args)
        if (++calls === 1) throw Error('response lost')
        return result
    }
    else h.transport.replaceFromTarget = async (...args) => {
        bodies.push(JSON.stringify(args))
        await (original as typeof h.transport.replaceFromTarget)(...args)
        if (++calls === 1) throw Error('response lost')
    }
    await expect(h.flow.bind(h.target, h.transport)).rejects.toThrow('response lost')
    expect(h.events).not.toContain('projection')
    expect(h.transport.resumeBinding).not.toHaveBeenCalled()
    expect((await h.runtime.retryCommittedWorkingSetRefresh())?.projection).toBe('applied')
    expect(bodies[1]).toBe(bodies[0])
    expect(h.transport.pullAvailableState).toHaveBeenCalledOnce()
    expect(h.transport.resumeBinding).toHaveBeenCalledOnce()
    expect(h.database.username).toBe('Activated synthetic target')
})

it('does not adopt or resume an uncertain activation after an unrelated native authority transition', async () => {
    const h = await harness()
    const switchTarget = h.dependencies.native.switchTarget
    h.dependencies.native.switchTarget = async (...args) => { await switchTarget(...args); throw Error('response lost') }
    await expect(h.flow.bind(h.target, h.transport)).rejects.toThrow('response lost')
    h.changeAuthority()
    expect((await h.runtime.retryCommittedWorkingSetRefresh())?.projection).toBe('refresh-required')
    expect(h.events).not.toContain('projection')
    expect(h.transport.resumeBinding).not.toHaveBeenCalled()
    expect(() => h.runtime.markPersistentDataDirty(1)).toThrow()
})

it.each(['request', 'target'])('rejects changed %s in an uncertain switch replay before projection', async change => {
    const h = await harness()
    const switchTarget = h.dependencies.native.switchTarget
    h.dependencies.native.switchTarget = async (...args) => { await switchTarget(...args); throw Error('response lost') }
    await expect(h.flow.bind(h.target, h.transport)).rejects.toThrow('response lost')
    h.dependencies.native.switchTarget = async (...args) => {
        if (change === 'request') return switchTarget(args[0], args[1], args[2], 'changed-request')
        return { ...await switchTarget(...args), target: { kind: 'external', connectionId: 'unrelated' } }
    }
    expect((await h.runtime.retryCommittedWorkingSetRefresh())?.projection).toBe('refresh-required')
    expect(h.events).not.toContain('projection')
    expect(h.transport.resumeBinding).not.toHaveBeenCalled()
})

it('settles explicit new-device activation with its original stage and authorization before specialized resume', async () => {
    const h = await harness()
    const replace = h.transport.replaceFromTarget
    const preparation = { authorizationId: 'native-authorization', writerId: 'reserved-writer' }
    h.transport.prepareNewDeviceBinding = vi.fn(async () => preparation)
    const bodies: string[] = []
    h.transport.replaceAsNewDevice = async (stage, receipt, context) => {
        bodies.push(JSON.stringify({ stage, receipt, state: context.state, mode: context.mode }))
        await replace(stage, context)
        h.setState({ target: h.target, targetAuthority: '1', selectionEpoch: 'new', libraryId: 'library', progress: null })
        if (bodies.length === 1) throw Error('response lost')
        return { revision: (await h.store.readRoot()).revision, writerId: 'reserved-writer', bindingAuthority: '1' }
    }
    h.transport.resumeNewDeviceBinding = vi.fn(async (receipt, result, context) => {
        expect(receipt).toEqual(preparation)
        expect(result.writerId).toBe('reserved-writer')
        expect(context.mode).toBe('new-device')
    })
    await expect(h.flow.bind(h.target, h.transport, { mode: 'new-device' })).rejects.toThrow('response lost')
    expect((await h.runtime.retryCommittedWorkingSetRefresh())?.projection).toBe('applied')
    expect(bodies[1]).toBe(bodies[0])
    expect(h.transport.prepareNewDeviceBinding).toHaveBeenCalledOnce()
    expect(h.transport.resumeNewDeviceBinding).toHaveBeenCalledOnce()
    expect(h.transport.resumeBinding).not.toHaveBeenCalled()
    expect(h.transport.publishInitialSharedState).not.toHaveBeenCalled()
})
