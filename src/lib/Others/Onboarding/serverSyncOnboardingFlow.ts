/**
 * What the sync-server screen of the onboarding shows once a connection has
 * started. The controller snapshot is the only input, so the rules can be
 * checked without a DOM.
 */

import type { ServerSyncSnapshot } from 'src/ts/storage/sync/serverSyncController'
import { completedServerSyncAttempt } from 'src/ts/storage/sync/serverSyncPresenter'

/** The screen before a connection starts: the code box, then the server check. */
export type ServerSyncOnboardingStage = 'code' | 'review' | 'syncing'

export type ServerSyncOnboardingOutcome =
    | 'syncing'
    | 'complete'
    | 'conflict'
    | 'paused'
    | 'pending'
    | 'error'

/**
 * Reads the attempt that the reader started from this screen. `complete`
 * leads to the last onboarding screen; `paused` does too, because the reader
 * stopped it and the settings can continue it. `pending` is a server job
 * that has not finished yet and can simply be retried.
 */
export function serverSyncOnboardingOutcome(
    snapshot: ServerSyncSnapshot,
): ServerSyncOnboardingOutcome {
    if (snapshot.running) return 'syncing'
    if (completedServerSyncAttempt(snapshot) !== null) return 'complete'
    if (snapshot.result?.phase === 'conflict') return 'conflict'
    if (snapshot.paused) return 'paused'
    if (!snapshot.error && snapshot.result?.phase === 'pending') return 'pending'
    return 'error'
}

/**
 * What the screen does when it opens on a device that is already connected,
 * where a registration code would be refused: draw the attempt that is
 * running or paused, or start another one. Nothing for a device that is not
 * connected.
 */
export function serverSyncOnboardingResume(
    snapshot: ServerSyncSnapshot | undefined,
): 'show' | 'retry' | undefined {
    if (!snapshot?.status?.configured) return undefined
    return snapshot.running || snapshot.paused ? 'show' : 'retry'
}
