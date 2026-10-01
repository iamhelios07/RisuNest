import { describe, expect, it, vi } from 'vitest'
import { createSyncBindingFlow, type InspectedSyncTarget, type SyncBindingDependencies, type SyncBindingState, type SyncBindingTransport } from './bindingFlow'

function setup(options: Partial<InspectedSyncTarget> & { nonDefault?: boolean; confirm?: boolean } = {}) {
    const events: string[] = []
    let state: SyncBindingState = { target: { kind: 'none' }, targetAuthority: '0', selectionEpoch: 'old', libraryId: null, progress: null }
    const record = (name: string) => async () => { events.push(name) }
    const deps: SyncBindingDependencies = {
        native: {
            state: async () => structuredClone(state),
            assertAuthority: async expected => { if (expected.targetAuthority !== state.targetAuthority) throw Error('stale authority') },
            switchTarget: async (expected, target, inspection) => {
                if (expected.targetAuthority !== state.targetAuthority) throw Error('stale authority')
                events.push('switch'); state = { ...state, target, libraryId: inspection?.libraryId ?? state.libraryId, targetAuthority: String(Number(state.targetAuthority) + 1) }; return state
            },
        },
        gate: { runTransition: async f => { events.push('gate'); return f() }, runWrite: f => f(), runKeyedWrite: (_key, f) => f() },
        plugins: { fenceExecution: record('plugin-fence'), invalidateCaches: record('invalidate'), restart: record('restart') },
        withPausedWrites: async operation => { events.push('pause-flush'); return operation() }, hasNonDefaultData: async () => options.nonDefault ?? true,
        hasNonDefaultSharedData: async () => options.nonDefault ?? true,
        confirmReplacement: async () => { events.push('confirm'); return options.confirm ?? true },
        refreshActivatedLibrary: record('refresh'),
    }
    const target: InspectedSyncTarget = { inspectionId: 'inspected', targetId: 'target', libraryId: 'library', empty: false, previouslyBoundLibrary: false, ...options }
    const transport: SyncBindingTransport = {
        inspectTarget: async () => { events.push('inspect'); return target },
        pullAvailableState: async () => { events.push('stage'); return { targetId: 'target', libraryId: 'library', stagingId: 'validated', receiveId: 'receive' } },
        replaceFromTarget: record('activate-database'), publishInitialSharedState: record('publish'), resumeBinding: record('resume'), fenceOldJobs: record('jobs-fence'),
    }
    return { events, deps, transport, flow: createSyncBindingFlow(deps), changeAuthority: () => { state.targetAuthority = 'changed' } }
}
const target = { kind: 'server', connectionId: 'connection' } as const

describe('transport-neutral first binding', () => {
    it('acknowledges, validates, fences and activates database before refreshing and restarting plugins', async () => {
        const s = setup()
        expect(await s.flow.bind(target, s.transport)).toMatchObject({ kind: 'bound', action: 'replaced' })
        expect(s.events).toEqual(['inspect', 'confirm', 'stage', 'jobs-fence', 'plugin-fence', 'pause-flush', 'gate', 'switch', 'activate-database', 'invalidate', 'refresh', 'restart'])
    })
    it('cancellation leaves original authority, plugin execution and contents intact', async () => {
        const s = setup({ confirm: false })
        expect(await s.flow.bind(target, s.transport)).toEqual({ kind: 'cancelled' })
        expect(s.events).toEqual(['inspect', 'confirm'])
    })
    it('default-only first binding replaces without a dialog', async () => {
        const s = setup({ nonDefault: false }); await s.flow.bind(target, s.transport)
        expect(s.events).not.toContain('confirm'); expect(s.events).toContain('activate-database')
    })
    it('failed staging never fences or clears any values', async () => {
        const s = setup(); s.transport.pullAvailableState = async () => { throw Error('invalid stage') }
        await expect(s.flow.bind(target, s.transport)).rejects.toThrow('invalid stage')
        expect(s.events).toEqual(['inspect', 'confirm'])
    })
    it('rejects a changed staged target without switching authority', async () => {
        const s = setup(); s.transport.pullAvailableState = async () => ({ targetId: 'wrong', libraryId: 'library', stagingId: 'stage', receiveId: 'receive' })
        await expect(s.flow.bind(target, s.transport)).rejects.toThrow('Sync target changed')
        expect(s.events).not.toContain('switch')
    })
    it('empty targets initialize without resetting plugin values', async () => {
        const s = setup({ empty: true }); await s.flow.bind(target, s.transport)
        expect(s.events).toEqual(['inspect', 'jobs-fence', 'pause-flush', 'gate', 'switch', 'publish'])
    })
    it('default shared state needs no transfer even with plugin-local-only data', async () => {
        const s = setup({ empty: true }); s.deps.hasNonDefaultSharedData = async () => false
        await s.flow.bind(target, s.transport)
        expect(s.events).not.toContain('publish'); expect(s.events).not.toContain('plugin-fence')
    })
    it('same-library recovery resumes without staging or resetting namespaces', async () => {
        const s = setup({ previouslyBoundLibrary: true }); await s.flow.bind(target, s.transport)
        expect(s.events).toEqual(['inspect', 'jobs-fence', 'pause-flush', 'gate', 'switch', 'resume'])
    })
    it('stale state stops before activation', async () => {
        const s = setup(); s.transport.pullAvailableState = async () => { s.changeAuthority(); return { targetId: 'target', libraryId: 'library', stagingId: 'stage', receiveId: 'receive' } }
        await expect(s.flow.bind(target, s.transport)).rejects.toThrow('stale authority')
        expect(s.events).not.toContain('activate-database')
    })
    it('replacement failure leaves plugins fenced and does not restart against partial data', async () => {
        const s = setup(); s.transport.replaceFromTarget = async () => { throw Error('interrupted') }
        await expect(s.flow.bind(target, s.transport)).rejects.toThrow('interrupted')
        expect(s.events).toContain('plugin-fence'); expect(s.events).not.toContain('restart')
    })
    it('switching aborts old transport completions and fences the old transport', async () => {
        const s = setup({ empty: true }); let signal: AbortSignal | undefined
        s.transport.publishInitialSharedState = async context => { signal = context.signal }
        await s.flow.bind(target, s.transport)
        const oldSignal = signal
        const other = { ...s.transport, fenceOldJobs: vi.fn(s.transport.fenceOldJobs) }
        await s.flow.bind({ kind: 'external', connectionId: 'other' }, other)
        expect(oldSignal?.aborted).toBe(true)
        expect(other.fenceOldJobs).not.toHaveBeenCalled()
    })
})

import { createStorageMutationGate, createInRealmStorageLockManager } from '../storageMutationGate'
it('flushes actual gated commits before transition and refreshes after exclusive release', async () => {
    const s = setup()
    const gate = createStorageMutationGate({ locks: createInRealmStorageLockManager() })
    let paused = false
    s.deps.gate = gate
    s.deps.withPausedWrites = async operation => {
        if (paused) throw Error('already paused')
        paused = true
        try {
            await gate.runWrite(async () => { s.events.push('gated-flush') })
            return await operation()
        } finally { paused = false }
    }
    s.deps.refreshActivatedLibrary = async () => {
        expect(paused).toBe(true)
        await gate.runWrite(async () => { s.events.push('gated-refresh') })
    }
    await s.flow.bind(target, s.transport)
    expect(s.events).toContain('gated-flush')
    expect(s.events).toContain('gated-refresh')
    expect(paused).toBe(false)
    await gate.runWrite(async () => {})
})
it('cancellation and activation failure release the real pause and storage gate', async () => {
    for (const fail of [false, true]) {
        const s = setup({ nonDefault: false })
        const gate = createStorageMutationGate({ locks: createInRealmStorageLockManager() })
        s.deps.gate = gate
        let paused = false
        s.deps.withPausedWrites = async operation => {
            paused = true
            try { return await operation() } finally { paused = false }
        }
        if (fail) s.transport.replaceFromTarget = async () => { throw Error('activation failed') }
        else {
            let read = 0
            s.deps.hasNonDefaultData = async () => ++read > 1
            s.deps.confirmReplacement = async () => false
        }
        if (fail) await expect(s.flow.bind(target, s.transport)).rejects.toThrow('activation failed')
        else expect(await s.flow.bind(target, s.transport)).toEqual({ kind: 'cancelled' })
        expect(paused).toBe(false)
        await gate.runWrite(async () => {})
    }
})

it('acknowledges plugin-local writes that finish while plugin execution is draining', async () => {
    const s = setup({ nonDefault: false })
    let data = false
    s.deps.hasNonDefaultData = async () => data
    s.deps.plugins.fenceExecution = async () => { s.events.push('plugin-fence'); data = true }
    s.deps.confirmReplacement = async () => { s.events.push('confirm'); return false }
    expect(await s.flow.bind(target, s.transport)).toEqual({ kind: 'cancelled' })
    expect(s.events).toContain('restart')
    expect(s.events).not.toContain('switch')
    expect(s.events).not.toContain('activate-database')
    expect(s.events.indexOf('confirm')).toBeGreaterThan(s.events.indexOf('plugin-fence'))
})

it('drains a plugin mutation using the same gate before entering exclusive activation', async () => {
    const s = setup({ nonDefault: false })
    const gate = createStorageMutationGate({ locks: createInRealmStorageLockManager() })
    s.deps.gate = gate
    let release!: () => void
    const drain = new Promise<void>(resolve => { release = resolve })
    let mutation: Promise<void> | undefined
    s.transport.pullAvailableState = async () => {
        mutation = gate.runWrite(async () => { await drain; s.events.push('plugin-mutation') })
        return { targetId: 'target', libraryId: 'library', stagingId: 'validated', receiveId: 'receive' }
    }
    s.deps.plugins.fenceExecution = async () => { release(); await mutation; s.events.push('plugin-fence') }
    await s.flow.bind(target, s.transport)
    expect(s.events.indexOf('plugin-mutation')).toBeLessThan(s.events.indexOf('activate-database'))
})
it('late cancellation restarts plugins through the same gate after draining without clearing data', async () => {
    const s = setup({ nonDefault: false })
    const gate = createStorageMutationGate({ locks: createInRealmStorageLockManager() })
    s.deps.gate = gate
    let changed = false
    s.deps.hasNonDefaultData = async () => changed
    s.deps.plugins.fenceExecution = async () => { changed = true }
    s.deps.plugins.restart = () => gate.runWrite(async () => { s.events.push('gated-restart') })
    s.deps.confirmReplacement = async () => false
    expect(await s.flow.bind(target, s.transport)).toEqual({ kind: 'cancelled' })
    expect(s.events).toContain('gated-restart')
    expect(s.events).not.toContain('switch')
})
it('resume and initial publish callbacks may use the same renderer gate', async () => {
    for (const empty of [false,true]) {
        const s = setup({ empty, previouslyBoundLibrary: !empty })
        const gate = createStorageMutationGate({ locks: createInRealmStorageLockManager() }); s.deps.gate = gate
        s.transport.resumeBinding = () => gate.runWrite(async () => { s.events.push('gated-resume') })
        s.transport.publishInitialSharedState = () => gate.runWrite(async () => { s.events.push('gated-publish') })
        await s.flow.bind(target,s.transport)
        expect(s.events).toContain(empty ? 'gated-publish' : 'gated-resume')
    }
})

it('drains coordinator-queued plugin mutations before acquiring pause', async () => {
    const s = setup({ nonDefault: false })
    let paused = false
    const queuedSetter = Promise.resolve().then(() => { if (paused) throw Error('setter blocked behind pause') })
    s.deps.plugins.fenceExecution = async () => { await queuedSetter; expect(paused).toBe(false) }
    s.deps.withPausedWrites = async operation => { paused = true; try { return await operation() } finally { paused = false } }
    await s.flow.bind(target,s.transport)
})
