import isEqual from 'lodash/isEqual'

import { safeStructuredClone } from '../polyfill'
import {
    cloneConversationMetadata,
    type ActiveConversationSession,
} from '../storage/activeConversationSession'
import type { WindowedConversationMutationController } from '../storage/activeWorkingSet.svelte'
import type { Chat } from '../storage/database.svelte'
import { cloneConversationMetadata as cloneMetadataFields } from '../storage/selectedConversationLifecycle'

type ConversationFields = Omit<Chat, 'message'>

export interface HistoryWindowSessionSource {
    session: ActiveConversationSession
    /** The conversation object whose message array the session owns. */
    conversation: Chat
}

export interface HistoryWindowControllerEnvironment {
    /** A windowed controller over `chat`, when the generation's conversation is windowed. */
    captureWindowed(chat: Chat, absoluteStartIndex: number): WindowedConversationMutationController | null
    /** The active session, when the generation's conversation is complete. */
    captureSession(): HistoryWindowSessionSource | null
    getCurrentSession(): ActiveConversationSession | null
    /** The conversation's live metadata, which other writers may change during the generation. */
    readLiveMetadata(): ConversationFields | null
    /** Called once when no backend holds the window any more. */
    onLost?(): void
}

export interface HistoryWindowController extends WindowedConversationMutationController {
    /**
     * Takes metadata changes made on the live conversation into the window and
     * publishes the window's own metadata changes.
     */
    reconcileMetadata(): boolean
}

function sessionHoldsWindow(
    source: HistoryWindowSessionSource,
    chat: Chat,
    absoluteStartIndex: number,
): boolean {
    const { session, conversation } = source
    if (!session.isActive || session.materializeCompatibilityArray() !== conversation.message) return false
    if (session.totalMessages !== absoluteStartIndex + chat.message.length) return false
    const messages = conversation.message
    for (let index = 0; index < chat.message.length; index += 1) {
        if (messages[absoluteStartIndex + index]?.chatId !== chat.message[index].chatId) return false
    }
    return true
}

/**
 * Writes a window of a complete conversation through its active session, by
 * absolute index. Capture fails unless the session ends with the window's
 * messages, identified by message id.
 */
export function captureSessionHistoryWindowController(
    source: HistoryWindowSessionSource,
    getCurrentSession: () => ActiveConversationSession | null,
    chat: Chat,
    absoluteStartIndex: number,
): WindowedConversationMutationController | null {
    const { session, conversation } = source
    if (
        !Number.isSafeInteger(absoluteStartIndex)
        || absoluteStartIndex < 0
        || getCurrentSession() !== session
        || !sessionHoldsWindow(source, chat, absoluteStartIndex)
    ) return null
    const pin = session.acquirePin('transaction')
    let released = false
    let expectedVersion = session.version
    const isCurrent = () => !released
        && session.isActive
        && getCurrentSession() === session
        && session.version === expectedVersion
        && session.materializeCompatibilityArray() === conversation.message
        && session.totalMessages === absoluteStartIndex + chat.message.length
    return {
        chat,
        absoluteStartIndex,
        isCurrent,
        applyRange(localStart, deleteCount, messages) {
            if (
                !isCurrent()
                || !Number.isSafeInteger(localStart)
                || localStart < 0
                || !Number.isSafeInteger(deleteCount)
                || deleteCount < 0
                || localStart + deleteCount > chat.message.length
            ) return false
            const detached = safeStructuredClone([...messages])
            session.applyOperation({
                expectedVersion,
                expectedMetadata: cloneConversationMetadata(conversation),
                metadata: cloneConversationMetadata(chat),
                ...(deleteCount === 0 && detached.length === 0
                    ? {}
                    : {
                        range: {
                            position: session.positionAt(absoluteStartIndex + localStart),
                            deleteCount,
                            messages: detached,
                        },
                    }),
            })
            chat.message.splice(localStart, deleteCount, ...safeStructuredClone(detached))
            expectedVersion = session.version
            return true
        },
        release() {
            if (released) return
            released = true
            pin.release()
        },
    }
}

function isPlainRecord(value: unknown): value is Record<string, unknown> {
    return typeof value === 'object' && value !== null && Object.getPrototypeOf(value) === Object.prototype
}

// Three-way merge against the metadata last published: a side that left a
// value as it was takes the other side's value, and the window wins when both
// changed it. Plain objects such as chat variables merge one level deeper.
function mergeField(window: unknown, baseline: unknown, live: unknown, nested: boolean): unknown {
    if (isEqual(window, baseline)) return live
    if (isEqual(live, baseline)) return window
    if (!nested && isPlainRecord(window) && isPlainRecord(live)) {
        const base = isPlainRecord(baseline) ? baseline : {}
        const merged: Record<string, unknown> = {}
        for (const key of new Set([...Object.keys(window), ...Object.keys(base), ...Object.keys(live)])) {
            const value = mergeField(window[key], base[key], live[key], true)
            if (value !== undefined) merged[key] = value
        }
        return merged
    }
    return window
}

function mergeLiveMetadata(chat: Chat, baseline: ConversationFields, live: ConversationFields): void {
    const fields = chat as unknown as Record<string, unknown>
    const base = baseline as Record<string, unknown>
    const current = live as Record<string, unknown>
    for (const key of new Set([...Object.keys(fields), ...Object.keys(base), ...Object.keys(current)])) {
        if (key === 'message') continue
        const value = mergeField(fields[key], base[key], current[key], false)
        if (value === undefined) delete fields[key]
        else if (value !== fields[key]) fields[key] = safeStructuredClone(value)
    }
}

/**
 * Keeps a generation's window writable across a promotion: when the current
 * backend goes stale, it captures whichever backend the conversation now
 * offers over the same window.
 */
export function createHistoryWindowController(
    environment: HistoryWindowControllerEnvironment,
    chat: Chat,
    absoluteStartIndex: number,
    initial: WindowedConversationMutationController | null = null,
): HistoryWindowController {
    let backend = initial
    let released = false
    let lost = false
    let baseline = cloneMetadataFields(chat)
    const resolve = (): WindowedConversationMutationController | null => {
        if (released) return null
        if (backend?.isCurrent()) return backend
        backend?.release()
        const source = environment.captureSession()
        backend = source
            ? captureSessionHistoryWindowController(
                source,
                environment.getCurrentSession,
                chat,
                absoluteStartIndex,
            )
            : environment.captureWindowed(chat, absoluteStartIndex)
        if (!backend && !lost) {
            lost = true
            environment.onLost?.()
        }
        return backend
    }
    const applyRange: WindowedConversationMutationController['applyRange'] = (
        localStart,
        deleteCount,
        messages,
        command,
    ) => {
        const current = resolve()
        const live = current ? environment.readLiveMetadata() : null
        if (!current || !live) return false
        mergeLiveMetadata(chat, baseline, live)
        if (!current.applyRange(localStart, deleteCount, messages, command)) return false
        baseline = cloneMetadataFields(chat)
        return true
    }
    return {
        chat,
        absoluteStartIndex,
        isCurrent: () => resolve() !== null,
        applyRange,
        reconcileMetadata() {
            const live = resolve() ? environment.readLiveMetadata() : null
            if (!live) return false
            mergeLiveMetadata(chat, baseline, live)
            const merged = cloneMetadataFields(chat)
            if (isEqual(merged, live)) {
                baseline = merged
                return true
            }
            return applyRange(0, 0, [], 'update-metadata')
        },
        release() {
            if (released) return
            released = true
            backend?.release()
            backend = null
        },
    }
}
