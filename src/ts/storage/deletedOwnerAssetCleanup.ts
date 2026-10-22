import { invoke } from '@tauri-apps/api/core'
import { isTauri } from '../platform'

const OWNED_PREFIX = 'assets/'
/** The longest key the native cleanup accepts. */
const MAX_KEY_LENGTH = 4096
/** A key inside longer text, such as an `<img src>` or a `{{raw::}}` reference. */
const EMBEDDED_KEY = /assets[\\/][^\s"'`<>()[\]{}|,;\\]+/g

function ownedKey(text: string): string | null {
    const key = text.replaceAll('\\', '/')
    return key.startsWith(OWNED_PREFIX) && key.length > OWNED_PREFIX.length && key.length <= MAX_KEY_LENGTH ? key : null
}

/** The `assets/` keys a character, module or persona names anywhere in its data. */
export function collectOwnedAssetKeys(...values: unknown[]): string[] {
    const keys = new Set<string>()
    const pending = [...values]
    while (pending.length) {
        const value = pending.pop()
        if (typeof value === 'string') {
            const whole = ownedKey(value)
            if (whole) keys.add(whole)
            for (const match of value.matchAll(EMBEDDED_KEY)) {
                const key = ownedKey(match[0])
                if (key) keys.add(key)
            }
        } else if (Array.isArray(value)) {
            pending.push(...value)
        } else if (value && typeof value === 'object') {
            pending.push(...Object.values(value))
        }
    }
    return [...keys]
}

/**
 * After a permanent delete is committed, deletes the assets only the deleted owner used.
 * `startedAtMs` is the wall-clock time the delete began, so an asset saved since is kept.
 * Pending edits are committed first so a reference that exists only in memory still counts.
 * Runs in the background; a failure leaves the assets to the unused-file cleanup.
 */
export function scheduleDeletedOwnerAssetCleanup(
    keys: readonly string[],
    startedAtMs: number,
    flush: () => Promise<void>,
): void {
    if (!isTauri || keys.length === 0) return
    void (async () => {
        await flush()
        await invoke('pds_asset_cleanup_deleted_owner', { keys, startedAtMs })
    })().catch((error) => console.error('Deleted owner asset cleanup failed', error))
}
