import { get } from 'svelte/store'
import { ReloadGUIPointer } from '../stores.svelte'

/** Chat variables that a script changed, or every variable for an explicit `reloadDisplay`. */
export type ScriptDisplayRefreshRequest = { keys: readonly string[] } | { all: true }

let pending: { all: boolean, keys: Set<string> } | null = null

/** Re-renders chat rows after a script changed what they display. Requests in one task coalesce. */
export function requestScriptDisplayRefresh(request: ScriptDisplayRefreshRequest): void {
    if (!('all' in request) && request.keys.length === 0) return
    if (!pending) {
        pending = { all: false, keys: new Set() }
        queueMicrotask(flushScriptDisplayRefresh)
    }
    if ('all' in request) pending.all = true
    else for (const key of request.keys) pending.keys.add(key)
}

function flushScriptDisplayRefresh(): void {
    pending = null
    ReloadGUIPointer.set(get(ReloadGUIPointer) + 1)
}
