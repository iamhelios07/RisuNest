import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { SyncBindingTransport, SyncBindingDependencies } from './bindingFlow'

const f = vi.hoisted(() => ({
    native: true,
    invoke: vi.fn(),
    install: vi.fn(),
    register: vi.fn(),
    paused: vi.fn(),
    guard: vi.fn(),
    refresh: vi.fn(),
    apply: vi.fn(),
    dispose: vi.fn(),
    transport: undefined as SyncBindingTransport | undefined,
    dependencies: undefined as Pick<SyncBindingDependencies, 'withPausedWrites' | 'beginActivatedLibraryGuard' | 'refreshActivatedLibrary'> | undefined,
}))
vi.mock('@tauri-apps/api/core', () => ({ invoke: f.invoke }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))
vi.mock('src/ts/platform', () => ({ get isTauri() { return f.native } }))
vi.mock('./bindingProduction', () => ({ installSyncBindingFlow: f.install }))
vi.mock('./bindingRegistry', () => ({ registerSyncBindingTransport: f.register, bindSyncTarget: vi.fn(), unbindSyncTarget: vi.fn() }))
vi.mock('./bindingNative', () => ({ replaceNativeSyncBinding: vi.fn(), replaceNativeSyncBindingAsNewDevice: vi.fn() }))
vi.mock('../persistentDataRuntime.svelte', () => ({
    withPausedPersistentWrites: f.paused,
    beginActivatedLibraryGuard: f.guard,
    refreshActivatedLibraryUnderPause: f.refresh,
    applyPersistentLwwReceive: f.apply,
    getPersistentDataRuntime: () => ({ subscribeActiveConversationViewportSource: () => () => {} }),
}))
vi.mock('src/ts/plugins/apiV3/v3.svelte', () => ({ fencePluginExecutionForAuthorityReplacement: vi.fn(), invalidatePluginCachesAfterAuthorityReplacement: vi.fn(), restartPluginsAfterAuthorityReplacement: vi.fn() }))
vi.mock('../persistentRevisionEvents', () => ({ subscribeLocalPersistentRevision: () => () => {} }))
vi.mock('../generatingConversationRegistry', () => ({ generatingConversations: { snapshot: () => [] } }))

let production: typeof import('./serverSyncProduction')
beforeEach(async () => {
    vi.resetModules(); vi.clearAllMocks(); vi.useFakeTimers()
    f.native = true; f.transport = undefined; f.dependencies = undefined
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
    it('guards native initialization and transport registration on the web', async () => {
        f.native = false
        production.initializeNativeSyncBindings(); await production.installServerSyncProduction()
        expect(f.install).not.toHaveBeenCalled(); expect(f.register).not.toHaveBeenCalled(); expect(f.invoke).not.toHaveBeenCalled()
    })
})
