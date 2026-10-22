import { ReloadChatPointer, ScriptDisplayRebuildPointer, ScriptDisplayRefreshPointer } from '../stores.svelte'
import { displayRowsToRefresh, markDisplayRowsForRebuild, type ScriptDisplayRefreshRequest } from './scriptVariableReads'

let pending: { all: boolean, keys: Set<string> } | null = null

/**
 * Re-renders the chat rows that read the chat variables a script changed. An explicit
 * `reloadDisplay` re-renders every mounted row and the background, rebuilding their DOM.
 * Requests in one task coalesce.
 */
export function requestScriptDisplayRefresh(request: ScriptDisplayRefreshRequest): void {
    if (!('all' in request) && request.keys.length === 0) return
    if (!pending) {
        pending = { all: false, keys: new Set() }
        queueMicrotask(flushScriptDisplayRefresh)
    }
    if ('all' in request) pending.all = true
    else for (const key of request.keys) pending.keys.add(key)
}

/** Re-renders the mounted rows at a message index now and rebuilds their DOM, for a script reload of that message. */
export function rebuildScriptDisplayRow(index: number): void {
    markDisplayRowsForRebuild(index)
    bumpReloadPointers([index])
}

function flushScriptDisplayRefresh(): void {
    const request = pending
    pending = null
    if (!request) return
    const indexes = request.all ? markDisplayRowsForRebuild() : displayRowsToRefresh({ keys: [...request.keys] })
    if (request.all) ScriptDisplayRebuildPointer.update((value) => value + 1)
    // Clears cached regex results and refreshes the background before rows render again.
    ScriptDisplayRefreshPointer.update((value) => value + 1)
    bumpReloadPointers(indexes)
}

function bumpReloadPointers(indexes: number[]): void {
    if (indexes.length === 0) return
    ReloadChatPointer.update((pointers) => {
        for (const index of indexes) pointers[index] = (pointers[index] ?? 0) + 1
        return pointers
    })
}
