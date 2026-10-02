import { invoke } from '@tauri-apps/api/core'
import type { SyncExitDrainAdapter } from '../syncExitCoordinator'
import type { RetainableReplacementFence } from '../retainableReplacementFence'
import { isTauri } from 'src/ts/platform'
import { installSyncBindingFlow } from './bindingProduction'
import { withPausedPersistentWrites, beginActivatedLibraryGuard, refreshActivatedLibraryUnderPause, applyPersistentLwwReceive, getPersistentDataRuntime } from '../persistentDataRuntime.svelte'
import type { PersistentMutationToken } from '../saveCoordinator'
import type { LwwStageReceive } from '../persistentDataStore'
import { fencePluginExecutionForAuthorityReplacement, invalidatePluginCachesAfterAuthorityReplacement, restartPluginsAfterAuthorityReplacement } from 'src/ts/plugins/apiV3/v3.svelte'
import { registerSyncBindingTransport } from './bindingRegistry'
import type { SyncBindingTransport, BindingContext } from './bindingFlow'
import { replaceNativeSyncBinding, replaceNativeSyncBindingAsNewDevice } from './bindingNative'
import { createServerSyncScheduler, serverSyncErrorCode } from './serverSyncScheduler'
import { subscribeLocalPersistentRevision } from '../persistentRevisionEvents'
import { listen } from '@tauri-apps/api/event'
import { generatingConversations } from '../generatingConversationRegistry'
import { bindSyncTarget, unbindSyncTarget } from './bindingRegistry'
import type { ServerConfig } from './serverSync'

let nativeBindings: ReturnType<typeof installSyncBindingFlow> | undefined
let disposeServer: (() => void) | undefined
export function initializeNativeSyncBindings(): () => void {
    if (!isTauri) return () => {}
    if (nativeBindings) return disposeNativeSyncBindings
    let activeToken: PersistentMutationToken | undefined
    const token = () => { if (!activeToken) throw new Error('Sync binding pause is unavailable'); return activeToken }
    nativeBindings = installSyncBindingFlow({
        withPausedWrites: operation => withPausedPersistentWrites('sync-binding', async pausedToken => {
            activeToken = pausedToken
            try { return await operation() } finally { activeToken = undefined }
        }),
        beginActivatedLibraryGuard: () => beginActivatedLibraryGuard(token()),
        refreshActivatedLibrary: async () => {
            const outcome = await refreshActivatedLibraryUnderPause(token())
            if (outcome.projection !== 'applied') throw new Error('Sync binding projection is unavailable')
        },
        plugins: { fenceExecution: fencePluginExecutionForAuthorityReplacement, invalidateCaches: invalidatePluginCachesAfterAuthorityReplacement, restart: restartPluginsAfterAuthorityReplacement },
    })
    return disposeNativeSyncBindings
}
export function disposeNativeSyncBindings(): void { disposeServer?.(); disposeServer = undefined; nativeBindings?.dispose(); nativeBindings = undefined }

type Status = { configured: boolean; libraryId?: string | null; deviceId?: string | null; writerId: string; bindingAuthority: string }
let status: Status = { configured: false, writerId: '', bindingAuthority: '0' }
let context: BindingContext | undefined
let error = ''
let foreground = false
let replacing = false
const listeners = new Set<(value: ReturnType<typeof snapshot>) => void>()
const changed = () => { for (const listener of listeners) listener(snapshot()) }
const header = () => {
    if (!context || context.signal.aborted) throw new Error('Sync binding is unavailable')
    return { bindingAuthority: context.state.targetAuthority, requestId: crypto.randomUUID() }
}
const scheduler = createServerSyncScheduler({
    async push() { const captured = context; if (!captured) return; for (;;) { captured.signal.throwIfAborted(); if (captured !== context) throw new Error('Sync binding changed'); const receipt = await invoke('server_sync_lww_push', { request: header(), generating: generatingConversations.snapshot() }); if (!receipt || !foreground) break } },
    async pull(completeAvailable = false) {
        const captured = context
        if (!captured) throw new Error('Sync binding is unavailable')
        for (;;) {
            captured.signal.throwIfAborted()
            if (captured !== context) throw new Error('Sync binding changed')
            const request = await invoke<LwwStageReceive>('server_sync_lww_pull', { request: header() })
            captured.signal.throwIfAborted()
            await applyPersistentLwwReceive(request)
            if (!foreground && !completeAvailable) break
            await invoke('server_sync_lww_ack', { request: { bindingAuthority: request.bindingAuthority, requestId: request.requestId } })
            if (request.changes.length === 0) break
        }
    },
    connect: () => context ? invoke('server_sync_notify_start', { request: header() }) : Promise.resolve(),
    disconnect: async () => { await invoke('server_sync_notify_stop'); await invoke('server_sync_cancel') },
    failed(value) { error = serverSyncErrorCode(value) || 'server-unreachable'; changed() },
})
const transport: SyncBindingTransport = {
    inspectTarget: c => invoke('server_sync_lww_inspect', { request: { bindingAuthority: c.state.targetAuthority, requestId: crypto.randomUUID() } }),
    pullAvailableState: (inspected,c) => invoke('server_sync_lww_stage_target', { inspectionId: inspected.inspectionId, request: { bindingAuthority: c.state.targetAuthority, requestId: crypto.randomUUID() } }),
    replaceFromTarget: replaceNativeSyncBinding,
    async fenceOldJobs(c) { foreground = false; await scheduler.fence(); await invoke('server_sync_lww_fence', { newDevice: c.mode === 'new-device' }) },
    async publishInitialSharedState(c) {
        context = c
        await invoke('server_sync_lww_activate', { request: header() })
        let afterKey: string | null = null
        for (;;) {
            const page = await invoke<{ afterKey: string | null; hasMore: boolean }>('pds_lww_queue_unit_state_page', { request: { ...header(), afterKey, limit: '256' } })
            if (!page.hasMore) break
            afterKey = page.afterKey
        }
        foreground = document.visibilityState !== 'hidden'
        await scheduler.foreground(foreground)
        await scheduler.flush()
    },
    async resumeBinding(c) { context = c; await invoke('server_sync_lww_activate', { request: header() }); foreground = document.visibilityState !== 'hidden'; await scheduler.foreground(foreground); scheduler.remoteHint(); if (foreground) void invoke('server_sync_lww_hydrate', { request: header() }).catch(() => {}) },
    prepareNewDeviceBinding: (staged,c) => invoke('server_sync_lww_prepare_new_device', { stagingId: staged.stagingId, request: { bindingAuthority: c.state.targetAuthority, requestId: staged.receiveId } }),
    replaceAsNewDevice: replaceNativeSyncBindingAsNewDevice,
    async resumeNewDeviceBinding(preparation,result,c) { await invoke('server_sync_lww_activate_new_device', { authorizationId: preparation.authorizationId, writerId: result.writerId, request: { bindingAuthority: result.bindingAuthority, requestId: crypto.randomUUID() } }); context = c; error = ''; foreground = document.visibilityState !== 'hidden'; await scheduler.foreground(foreground); await scheduler.retry(); scheduler.remoteHint() },
}
export async function receiveAvailableServerChanges(): Promise<void> { await scheduler.receiveAvailableChanges() }
Object.assign(transport, { receiveAvailableChanges: async (c: BindingContext) => { c.signal.throwIfAborted(); if (!context || context.state.targetAuthority !== c.state.targetAuthority || context.state.selectionEpoch !== c.state.selectionEpoch) throw new Error('Sync binding changed'); await receiveAvailableServerChanges(); c.signal.throwIfAborted() } })
export async function connectServerSync(config: ServerConfig, newDevice = false): Promise<void> { await invoke('server_sync_configure', { config }); error = ''; await bindSyncTarget({ kind: 'server', connectionId: 'server' }, newDevice ? { mode: 'new-device' } : {}); await controller.ensureStatus() }
export async function disconnectServerSync(): Promise<void> { await unbindSyncTarget(); context = undefined; await controller.ensureStatus() }
export async function retryServerSync(): Promise<void> { await scheduler.fence(); await invoke('server_sync_lww_retry', { request: header() }); error = ''; await scheduler.foreground(foreground = document.visibilityState !== 'hidden'); await scheduler.retry(); changed() }
export async function installServerSyncProduction(): Promise<void> {
    if (!isTauri || disposeServer) return
    const disposers = [registerSyncBindingTransport({ kind: 'server', connectionId: 'server' }, transport)]
    disposers.push(subscribeLocalPersistentRevision((_revision,cause) => scheduler.localChange(cause === 'generation-complete')))
    disposers.push(getPersistentDataRuntime().subscribeActiveConversationViewportSource(() => scheduler.conversationOpened()))
    disposers.push(await listen('risu-server-sync-remote-hint', () => scheduler.remoteHint()))
    disposers.push(await listen<{ connected: boolean }>('risu-server-sync-notification', event => scheduler.socket(event.payload.connected)))
    const visibility = () => { foreground = document.visibilityState !== 'hidden'; void scheduler.foreground(foreground).catch(value => { error = String(value); changed() }) }
    document.addEventListener('visibilitychange', visibility)
    disposers.push(() => document.removeEventListener('visibilitychange', visibility))
    disposeServer = () => { scheduler.dispose(); for (const dispose of disposers) dispose(); context = undefined }
    await controller.ensureStatus()
    const current = await invoke<BindingContext['state']>('pds_lww_binding_state')
    if (current.target.kind === 'server' && status.configured) await transport.resumeBinding({ state: current, signal: new AbortController().signal })
}

const snapshot = () => ({ status: { ...status, bound: !!context }, running: scheduler.isRunning(), paused: !context || scheduler.isBlocked(), replacing, draining: false, error })
const controller = {
    snapshot,
    subscribe: (listener: (value: ReturnType<typeof snapshot>) => void) => { listeners.add(listener); listener(controller.snapshot()); return () => { listeners.delete(listener) } },
    assertFileOperationAvailable: () => { if (replacing) throw new Error('server-sync-busy') },
    beginReplacement: async () => { replacing = true; foreground = false; await scheduler.fence(); changed(); return async () => { replacing = false; await scheduler.foreground(foreground = document.visibilityState !== 'hidden'); changed() } },
    confirmReplacement: async () => {},
    holdAutomaticSync: () => { foreground = false; void scheduler.fence() },
    ensureStatus: async () => { if (isTauri) status = await invoke<Status>('server_sync_status'); changed() },
}
export function getServerSyncController() { return controller }
export function createServerSyncExitDrainAdapter(id = 'server', _fence?: RetainableReplacementFence): SyncExitDrainAdapter {
    return { id, async drain(target, signal) {
        const cancel = () => { void invoke('server_sync_cancel') }
        signal.addEventListener('abort', cancel, { once: true })
        try {
            signal.throwIfAborted()
            if (!context || context.state.selectionEpoch !== target.selectionEpoch) return { kind: 'blocked', reason: 'binding-authority-changed' }
            await scheduler.fence()
            await invoke('server_sync_lww_drain', { request: header(), generating: generatingConversations.snapshot() })
            signal.throwIfAborted()
            return { kind: 'complete' }
        } catch (value) { return { kind: 'blocked', reason: typeof value === 'object' && value && 'code' in value ? String(value.code) : 'server-unreachable' } }
        finally { signal.removeEventListener('abort', cancel) }
    }, cancel: async () => { foreground = false; await scheduler.fence() }, resumeAfterExitCancel: async () => { await scheduler.foreground(foreground = document.visibilityState !== 'hidden') } }
}
export function holdServerSyncAfterRestore(): void { controller.holdAutomaticSync() }
export function resumeServerSyncAfterBackup(): void { if (context) void scheduler.foreground(foreground = document.visibilityState !== 'hidden') }
export interface ServerSyncCacheUsage {
  /** cacheBytes plus ledgerBytes. */
  totalBytes: number;
  /** The temporary files: protectedBytes plus reclaimableBytes. */
  cacheBytes: number;
  protectedBytes: number;
  reclaimableBytes: number;
  /** The asset residency ledger kept beside the temporary files. */
  ledgerBytes: number;
  /** Part of cacheBytes: what the cache's object database occupies on disk. */
  databaseBytes: number;
  blockedReason: string | null;
}

export const getServerSyncCacheUsage = () => invoke<ServerSyncCacheUsage>('server_sync_cache_usage')
export const cleanupServerSyncCache = () => invoke<ServerSyncCacheUsage>('server_sync_cache_cleanup')
