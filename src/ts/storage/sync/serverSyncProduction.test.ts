import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { SyncBindingTransport, SyncBindingDependencies } from './bindingFlow'

const f = vi.hoisted(() => ({
    native: true,
    selectedIndex: 0,
    database: { characters: [{ chaId: 'selected-stable-id' }] },
    invoke: vi.fn(),
    install: vi.fn(),
    register: vi.fn(),
    paused: vi.fn(),
    guard: vi.fn(),
    refresh: vi.fn(),
    apply: vi.fn(),
    dispose: vi.fn(),
    continuation: vi.fn(),
    generating: [] as Array<{ characterId: string; conversationId: string }>,
    runtime: {
        revision: 31,
        getStorageAuthorityEpoch: vi.fn(() => 'storage-epoch'),
        subscribeActiveConversationViewportSource: () => () => {},
        setActivatedLibraryRecoveryLifecycle: vi.fn(),
        markCommittedWorkingSetRefreshRequired: vi.fn(),
    },
    transport: undefined as SyncBindingTransport | undefined,
    dependencies: undefined as Pick<SyncBindingDependencies, 'withPausedWrites' | 'beginActivatedLibraryGuard' | 'refreshActivatedLibrary' | 'recovery'> | undefined,
}))
vi.mock('@tauri-apps/api/core', () => ({ invoke: f.invoke }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))
vi.mock('src/ts/platform', () => ({ get isTauri() { return f.native } }))
vi.mock('src/ts/stores.svelte', () => ({ selectedCharID: { subscribe: (run: (value: number) => void) => { run(f.selectedIndex); return () => {} } } }))
vi.mock('../database.svelte', () => ({ getDatabase: () => f.database }))
vi.mock('./bindingProduction', async importOriginal => ({
    ...await importOriginal<typeof import('./bindingProduction')>(),
    installSyncBindingFlow: f.install,
}))
vi.mock('../committedWorkingSetContinuation', () => ({ registerCommittedWorkingSetContinuation: f.continuation }))
vi.mock('./bindingDialog', () => ({ confirmSyncBindingReplacement: vi.fn() }))
vi.mock('./bindingLocalData', () => ({ hasLocalBindingData: vi.fn(), hasLocalSharedBindingData: vi.fn() }))
vi.mock('./bindingRegistry', () => ({ registerSyncBindingTransport: f.register, bindSyncTarget: vi.fn(), unbindSyncTarget: vi.fn() }))
vi.mock('./bindingNative', () => ({ replaceNativeSyncBinding: vi.fn(), replaceNativeSyncBindingAsNewDevice: vi.fn() }))
vi.mock('../persistentDataRuntime.svelte', () => ({
    withPausedPersistentWrites: f.paused,
    beginActivatedLibraryGuard: f.guard,
    refreshActivatedLibraryUnderPause: f.refresh,
    applyPersistentLwwReceive: f.apply,
    getPersistentDataRuntime: () => f.runtime,
}))
vi.mock('src/ts/plugins/apiV3/v3.svelte', () => ({ fencePluginExecutionForAuthorityReplacement: vi.fn(), invalidatePluginCachesAfterAuthorityReplacement: vi.fn(), restartPluginsAfterAuthorityReplacement: vi.fn() }))
vi.mock('../persistentRevisionEvents', () => ({ subscribeLocalPersistentRevision: () => () => {} }))
vi.mock('../generatingConversationRegistry', () => ({ generatingConversations: { snapshot: () => f.generating } }))

let production: typeof import('./serverSyncProduction')
beforeEach(async () => {
    vi.resetModules(); vi.clearAllMocks(); vi.useFakeTimers()
    f.native = true; f.transport = undefined; f.dependencies = undefined; f.generating = []; f.selectedIndex = 0
    const document = new EventTarget() as EventTarget & { visibilityState: string }
    document.visibilityState = 'hidden'
    vi.stubGlobal('document', document)
    f.invoke.mockImplementation(async command => command === 'server_sync_status'
        ? { configured: false, writerId: 'writer', bindingAuthority: '0' }
        : command === 'pds_lww_binding_state'
            ? { target: { kind: 'none' }, targetAuthority: '0', selectionEpoch: '0', libraryId: null, progress: null }
            : null)
    f.install.mockImplementation(dependencies => { f.dependencies = dependencies; return { dispose: f.dispose } })
    f.register.mockImplementation((_target, transport) => { f.transport = transport; return () => {} })
    f.refresh.mockResolvedValue({ projection: 'applied' })
    production = await import('./serverSyncProduction')
})
afterEach(() => { production.disposeNativeSyncBindings(); vi.useRealTimers(); vi.unstubAllGlobals() })

describe('production server LWW composition', () => {
    it('passes the actual stable selected character ID to foreground hydration and null when no character is selected', async () => {
        production.initializeNativeSyncBindings(); await production.installServerSyncProduction()
        const state = { target: { kind: 'server' as const, connectionId: 'server' }, targetAuthority: '0', selectionEpoch: '0', libraryId: 'library', progress: null }
        const context = { state, signal: new AbortController().signal }
        Object.defineProperty(document, 'visibilityState', { value: 'visible', configurable: true })
        await f.transport!.resumeBinding(context)
        expect(f.invoke).toHaveBeenCalledWith('server_sync_lww_hydrate', { request: expect.objectContaining({ bindingAuthority: '0' }), selectedCharacterId: 'selected-stable-id' })
        f.selectedIndex = -1
        await f.transport!.resumeBinding(context)
        expect(f.invoke).toHaveBeenCalledWith('server_sync_lww_hydrate', { request: expect.objectContaining({ bindingAuthority: '0' }), selectedCharacterId: null })
    })
    it('installs one shared binding flow without any server configuration and carries the exact pause token', async () => {
        const token = Object.freeze({ id: 'exact-token' })
        f.paused.mockImplementation(async (_reason, operation) => operation(token))
        production.initializeNativeSyncBindings(); production.initializeNativeSyncBindings()
        expect(f.install).toHaveBeenCalledTimes(1); expect(f.invoke).not.toHaveBeenCalled()
        await f.dependencies!.withPausedWrites(async () => {
            f.dependencies!.beginActivatedLibraryGuard()
            await f.dependencies!.refreshActivatedLibrary()
        })
        expect(f.guard).toHaveBeenCalledWith(token); expect(f.refresh).toHaveBeenCalledWith(token)
        expect(() => f.dependencies!.beginActivatedLibraryGuard()).toThrow('pause is unavailable')
        f.refresh.mockResolvedValueOnce({ projection: 'superseded' })
        await expect(f.dependencies!.withPausedWrites(() => f.dependencies!.refreshActivatedLibrary())).rejects.toThrow('projection is unavailable')
    })
    it('registers the server adapter with no registration, and waits for native receive application before ACK', async () => {
        production.initializeNativeSyncBindings(); await production.installServerSyncProduction()
        expect(f.register).toHaveBeenCalledTimes(1)
        const state = { target: { kind: 'server' as const, connectionId: 'server' }, targetAuthority: '0', selectionEpoch: '0', libraryId: 'library', progress: null }
        const context = { state, signal: new AbortController().signal }
        await f.transport!.resumeBinding(context)
        let complete!: () => void
        f.apply.mockImplementationOnce(() => new Promise<void>(resolve => { complete = resolve }))
        f.invoke.mockImplementation(async command => command === 'server_sync_lww_pull'
            ? { bindingAuthority: '0', requestId: 'received-page', changes: [], progress: { kind: 'server', cursor: '0' }, admittedTimeUpperMs: '0' }
            : null)
        const received = production.receiveAvailableServerChanges()
        await vi.advanceTimersByTimeAsync(0)
        expect(f.apply).toHaveBeenCalledTimes(1)
        expect(f.invoke.mock.calls.some(([command]) => command === 'server_sync_lww_ack')).toBe(false)
        complete(); await received
        expect(f.invoke).toHaveBeenCalledWith('server_sync_lww_ack', { request: { bindingAuthority: '0', requestId: 'received-page' } })
    })
    it('composes actual recovery registration with the exact pause token and a critical authority-bound continuation', async () => {
        const token = Object.freeze({ id: 'recovery-token' })
        const lifecycle = { beforeRefresh: vi.fn(async () => {}), afterRefresh: vi.fn(async () => {}) }
        const error = new Error('plugin restart failed')
        const resume = vi.fn(async () => {})
        f.paused.mockImplementation(async (_reason, operation) => operation(token))
        production.initializeNativeSyncBindings()
        await f.dependencies!.withPausedWrites(async () => {
            f.dependencies!.recovery!.setLifecycle(lifecycle)
        })
        expect(f.runtime.setActivatedLibraryRecoveryLifecycle).toHaveBeenCalledWith(token, lifecycle)
        expect(() => f.dependencies!.recovery!.setLifecycle(lifecycle)).toThrow('pause is unavailable')
        f.dependencies!.recovery!.registerFailure(error, resume)
        expect(f.runtime.markCommittedWorkingSetRefreshRequired).toHaveBeenCalledWith(31, error)
        expect(f.continuation).toHaveBeenCalledWith(31, f.runtime, 'storage-epoch', resume, undefined, true)
    })
    it('drains all 640 eligible initial units before starting foreground services', async () => {
        Object.defineProperty(document, 'visibilityState', { value: 'visible' })
        production.initializeNativeSyncBindings(); await production.installServerSyncProduction()
        const context = { state: { target: { kind: 'server' as const, connectionId: 'server' }, targetAuthority: '0', selectionEpoch: '0', libraryId: 'library', progress: null }, signal: new AbortController().signal }
        const units = Array.from({ length: 640 }, (_, index) => `synthetic-unit-${index}`)
        const pending: string[] = []
        const batches: number[] = []
        let queued = 0
        f.invoke.mockImplementation(async command => {
            if (command === 'pds_lww_queue_unit_state_page') {
                pending.push(...units.slice(queued, queued + 256))
                queued = Math.min(units.length, queued + 256)
                return { afterKey: units[queued - 1], hasMore: queued < units.length }
            }
            if (command === 'server_sync_lww_push') {
                const batch = pending.splice(0, 256)
                if (!batch.length) return null
                batches.push(batch.length)
                return { operationId: `native-batch-${batches.length}` }
            }
            return null
        })
        await f.transport!.publishInitialSharedState(context)
        expect(queued).toBe(640); expect(pending).toEqual([]); expect(batches).toEqual([256, 256, 128])
        expect(f.invoke.mock.calls.filter(([command]) => command === 'server_sync_lww_push')).toHaveLength(4)
        expect(f.invoke.mock.calls.some(([command]) => command === 'server_sync_lww_pull' || command === 'server_sync_notify_start')).toBe(false)
    })
    it('stops initial publication at a generating-only null batch and leaves service start to G', async () => {
        Object.defineProperty(document, 'visibilityState', { value: 'visible' })
        f.generating = [{ characterId: 'synthetic-character', conversationId: 'synthetic-chat' }]
        production.initializeNativeSyncBindings(); await production.installServerSyncProduction()
        const context = { state: { target: { kind: 'server' as const, connectionId: 'server' }, targetAuthority: '0', selectionEpoch: '0', libraryId: 'library', progress: null }, signal: new AbortController().signal }
        f.invoke.mockImplementation(async command => command === 'pds_lww_queue_unit_state_page' ? { afterKey: null, hasMore: false } : null)
        await f.transport!.publishInitialSharedState(context)
        const pushes = f.invoke.mock.calls.filter(([command]) => command === 'server_sync_lww_push')
        expect(pushes).toHaveLength(1); expect(pushes[0][1]).toMatchObject({ generating: f.generating })
        expect(f.invoke.mock.calls.some(([command]) => command === 'server_sync_lww_pull' || command === 'server_sync_notify_start')).toBe(false)
    })
    it('guards native initialization and transport registration on the web', async () => {
        f.native = false
        production.initializeNativeSyncBindings(); await production.installServerSyncProduction()
        expect(f.install).not.toHaveBeenCalled(); expect(f.register).not.toHaveBeenCalled(); expect(f.invoke).not.toHaveBeenCalled()
    })
})
