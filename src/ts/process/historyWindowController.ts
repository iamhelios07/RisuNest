import { safeStructuredClone } from '../polyfill'
import {
    cloneConversationMetadata,
    type ActiveConversationSession,
} from '../storage/activeConversationSession'
import type { WindowedConversationMutationController } from '../storage/activeWorkingSet.svelte'
import type { Chat } from '../storage/database.svelte'

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
): WindowedConversationMutationController {
    let backend = initial
    let released = false
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
        return backend
    }
    return {
        chat,
        absoluteStartIndex,
        isCurrent: () => resolve() !== null,
        applyRange: (localStart, deleteCount, messages, command) =>
            resolve()?.applyRange(localStart, deleteCount, messages, command) ?? false,
        release() {
            if (released) return
            released = true
            backend?.release()
            backend = null
        },
    }
}
