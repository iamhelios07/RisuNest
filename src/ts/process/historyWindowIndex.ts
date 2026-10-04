import type { Chat, Message } from '../storage/database.svelte'

// A generation that loads only the newest messages hands scripts a chat whose
// array starts at an absolute index. Script surfaces translate their absolute
// indices through these helpers; a chat without a window keeps array indices.
const windowStarts = new WeakMap<object, number>()

export function attachHistoryWindow(chat: Chat, absoluteStart: number): void {
    if (!Number.isSafeInteger(absoluteStart) || absoluteStart < 0) {
        throw new RangeError('History window start must be a non-negative integer')
    }
    windowStarts.set(chat, absoluteStart)
}

export function detachHistoryWindow(chat: Chat): void {
    windowStarts.delete(chat)
}

export function inheritHistoryWindow(source: Chat | null | undefined, target: Chat): void {
    const start = source ? windowStarts.get(source) : undefined
    if (start !== undefined) windowStarts.set(target, start)
}

export function getHistoryWindowStart(chat: Chat | null | undefined): number | null {
    return chat ? windowStarts.get(chat) ?? null : null
}

/** Number of messages in the whole conversation, loaded or not. */
export function historyLength(chat: Chat): number {
    return (windowStarts.get(chat) ?? 0) + chat.message.length
}

/**
 * Array index for an absolute message index. Indices before the window come
 * back negative, so a lookup with them finds nothing, as an out-of-range
 * index does without a window.
 */
export function toWindowIndex(chat: Chat, absoluteIndex: number): number {
    return absoluteIndex - (windowStarts.get(chat) ?? 0)
}

export function toAbsoluteIndex(chat: Chat, windowIndex: number): number {
    return windowIndex + (windowStarts.get(chat) ?? 0)
}

export function getHistoryMessage(chat: Chat, absoluteIndex: number): Message | undefined {
    const index = toWindowIndex(chat, absoluteIndex)
    return index >= 0 ? chat.message[index] : undefined
}

/** `Array.prototype.at` over the whole conversation. */
export function getHistoryMessageAt(chat: Chat, index: number): Message | undefined {
    const truncated = Math.trunc(Number(index))
    const integer = Number.isNaN(truncated) ? 0 : truncated
    if (integer < 0) return chat.message.at(integer)
    const windowIndex = toWindowIndex(chat, integer)
    return windowIndex >= 0 ? chat.message[windowIndex] : undefined
}

function normalizeSliceIndex(value: number | undefined, length: number, fallback: number): number {
    if (value === undefined) return fallback
    const integer = Math.trunc(Number(value))
    if (Number.isNaN(integer)) return 0
    if (integer < 0) return Math.max(length + integer, 0)
    return Math.min(integer, length)
}

/**
 * `Array.prototype.slice` over the whole conversation, returning only the
 * loaded messages that fall inside the requested range.
 */
export function sliceHistory(chat: Chat, start?: number, end?: number): Message[] {
    const offset = windowStarts.get(chat)
    if (offset === undefined) return chat.message.slice(start, end)
    const length = offset + chat.message.length
    const absoluteStart = normalizeSliceIndex(start, length, 0)
    const absoluteEnd = normalizeSliceIndex(end, length, length)
    return chat.message.slice(
        Math.max(absoluteStart - offset, 0),
        Math.max(absoluteEnd - offset, 0),
    )
}
