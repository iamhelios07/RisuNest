import type { WindowedConversationMutationController } from '../storage/activeWorkingSet.svelte'
import type { Message } from '../storage/database.svelte'

export interface HistoryWindowChange {
    start: number
    deleteCount: number
    messages: Message[]
}

function sameMessage(left: Message, right: Message): boolean {
    return left === right || JSON.stringify(left) === JSON.stringify(right)
}

/** The single range that turns `before` into `after`, or null when they match. */
export function diffHistoryWindow(
    before: readonly Message[],
    after: readonly Message[],
): HistoryWindowChange | null {
    const shared = Math.min(before.length, after.length)
    let prefix = 0
    while (prefix < shared && sameMessage(before[prefix], after[prefix])) prefix += 1
    if (prefix === before.length && prefix === after.length) return null
    let suffix = 0
    while (
        suffix < shared - prefix
        && sameMessage(before[before.length - 1 - suffix], after[after.length - 1 - suffix])
    ) suffix += 1
    return {
        start: prefix,
        deleteCount: before.length - prefix - suffix,
        messages: after.slice(prefix, after.length - suffix),
    }
}

/** Writes a script's edited window back through the controller as one range. */
export function writeHistoryWindow(
    controller: WindowedConversationMutationController,
    after: readonly Message[],
): boolean {
    const before = controller.chat.message
    const change = diffHistoryWindow(before, after)
    if (!change) return true
    const command = change.deleteCount === 0 && change.start === before.length
        ? 'append'
        : change.deleteCount === 1 && change.messages.length === 1
        ? 'edit'
        : 'replace-range'
    return controller.applyRange(change.start, change.deleteCount, change.messages, command)
}
