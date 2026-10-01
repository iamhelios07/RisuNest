import { invoke } from '@tauri-apps/api/core'
import type { SyncExitDrainAdapter } from '../syncExitCoordinator'
import type { RetainableReplacementFence } from '../retainableReplacementFence'

const snapshot = () => ({ status: { configured: false }, running: false, paused: true, replacing: false, draining: false, error: '' })
const controller = {
    snapshot,
    subscribe: (listener: (value: ReturnType<typeof snapshot>) => void) => { listener(controller.snapshot()); return () => {} },
    assertFileOperationAvailable: () => {},
    beginReplacement: async () => async () => {},
    confirmReplacement: async () => {},
    holdAutomaticSync: () => {},
    ensureStatus: async () => {},
}
export function getServerSyncController() { return controller }
export function createServerSyncExitDrainAdapter(id = 'server', _fence?: RetainableReplacementFence): SyncExitDrainAdapter {
    return { id, drain: async () => ({ kind: 'blocked', reason: 'sync-unavailable' }), cancel: async () => {} }
}
export function holdServerSyncAfterRestore(): void {}
export function resumeServerSyncAfterBackup(): void {}
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
