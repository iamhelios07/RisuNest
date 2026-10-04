import { describe, expect, it, vi } from 'vitest'

import { ActiveConversationSession } from '../storage/activeConversationSession'
import type { WindowedConversationMutationController } from '../storage/activeWorkingSet.svelte'
import type { Chat, Message } from '../storage/database.svelte'
import { cloneConversationMetadata } from '../storage/selectedConversationLifecycle'
import {
    captureSessionHistoryWindowController,
    createHistoryWindowController,
    type HistoryWindowSessionSource,
} from './historyWindowController'

const message = (data: string): Message => ({ role: 'user', data, chatId: `id-${data}` })

function completeConversation(count: number) {
    const conversation: Chat = {
        id: 'conversation-a',
        name: 'Conversation A',
        note: '',
        localLore: [],
        message: Array.from({ length: count }, (_, index) => message(`m${index}`)),
    }
    const session = new ActiveConversationSession({
        characterId: 'character-a',
        conversationId: 'conversation-a',
        conversation,
        storeRevision: 23,
    })
    // The session owns this object and swaps its message array on each write.
    const source: HistoryWindowSessionSource = { session, conversation }
    return { session, source }
}

function windowFrom(source: HistoryWindowSessionSource, start: number): Chat {
    const { message: messages, ...metadata } = source.conversation
    return { ...structuredClone(metadata), message: structuredClone(messages.slice(start)) }
}

describe('captureSessionHistoryWindowController', () => {
    it('writes by absolute index and leaves messages before the window untouched', () => {
        const { session, source } = completeConversation(10)
        const before = structuredClone(source.conversation.message.slice(0, 6))
        const chat = windowFrom(source, 6)
        const controller = captureSessionHistoryWindowController(source, () => session, chat, 6)!

        expect(controller.applyRange(1, 1, [{ ...chat.message[1], data: 'edited' }], 'edit')).toBe(true)
        expect(controller.applyRange(4, 0, [message('reply')], 'append')).toBe(true)
        expect(controller.applyRange(0, 1, [], 'replace-range')).toBe(true)

        const after = session.materializeCompatibilityArray()
        expect(after.slice(0, 6)).toEqual(before)
        expect(after.slice(6).map((item) => item.data)).toEqual(['edited', 'm8', 'm9', 'reply'])
        expect(chat.message.map((item) => item.data)).toEqual(['edited', 'm8', 'm9', 'reply'])
        expect(controller.isCurrent()).toBe(true)
    })

    it('publishes window metadata with a write and with a metadata-only update', () => {
        const { session, source } = completeConversation(4)
        const chat = windowFrom(source, 2)
        const controller = captureSessionHistoryWindowController(source, () => session, chat, 2)!

        chat.scriptstate = { $mood: 'calm' }
        expect(controller.applyRange(0, 0, [], 'update-metadata')).toBe(true)
        expect(session.version).toBe(1)
        expect(source.conversation.message).toBe(session.materializeCompatibilityArray())
        expect(controller.isCurrent()).toBe(true)
    })

    it('refuses a window whose ids or length do not match the session tail', () => {
        const { session, source } = completeConversation(6)
        const shifted = windowFrom(source, 3)
        shifted.message[1].chatId = 'other'
        expect(captureSessionHistoryWindowController(source, () => session, shifted, 3)).toBeNull()

        const short = windowFrom(source, 3)
        short.message.pop()
        expect(captureSessionHistoryWindowController(source, () => session, short, 3)).toBeNull()
        expect(captureSessionHistoryWindowController(source, () => null, windowFrom(source, 3), 3)).toBeNull()
    })

    it('goes stale after another write and holds a pin until released', () => {
        const { session, source } = completeConversation(5)
        const chat = windowFrom(source, 2)
        const controller = captureSessionHistoryWindowController(source, () => session, chat, 2)!

        expect(session.pinCount('transaction')).toBe(1)
        session.append(message('other'))
        expect(controller.isCurrent()).toBe(false)
        expect(controller.applyRange(0, 0, [message('late')], 'append')).toBe(false)
        controller.release()
        expect(session.pinCount('transaction')).toBe(0)
    })
})

function staleable(chat: Chat, absoluteStartIndex: number) {
    let current = true
    const applyRange = vi.fn((start: number, deleteCount: number, replacement: readonly Message[]) => {
        if (!current) return false
        chat.message.splice(start, deleteCount, ...structuredClone([...replacement]))
        return true
    })
    const controller: WindowedConversationMutationController = {
        chat,
        absoluteStartIndex,
        isCurrent: () => current,
        applyRange,
        release: vi.fn(),
    }
    return { controller, applyRange, expire: () => { current = false } }
}

describe('createHistoryWindowController', () => {
    it('carries writes over to the session after a promotion', () => {
        const { session, source } = completeConversation(8)
        const chat = windowFrom(source, 5)
        const windowed = staleable(chat, 5)
        let promoted = false
        const controller = createHistoryWindowController({
            captureWindowed: () => null,
            captureSession: () => promoted ? source : null,
            getCurrentSession: () => session,
            readLiveMetadata: () => cloneConversationMetadata(source.conversation),
        }, chat, 5, windowed.controller)

        expect(controller.applyRange(0, 1, [{ ...chat.message[0], data: 'edited' }], 'edit')).toBe(true)
        expect(windowed.applyRange).toHaveBeenCalledTimes(1)
        // Promotion reads the store, which holds what the windowed backend wrote.
        session.edit(session.locate(5), { ...session.materializeCompatibilityArray()[5], data: 'edited' })
        windowed.expire()
        promoted = true

        expect(controller.isCurrent()).toBe(true)
        expect(windowed.controller.release).toHaveBeenCalled()
        expect(controller.applyRange(3, 0, [message('reply')], 'append')).toBe(true)
        expect(session.materializeCompatibilityArray().map((item) => item.data))
            .toEqual(['m0', 'm1', 'm2', 'm3', 'm4', 'edited', 'm6', 'm7', 'reply'])
    })

    it('fails when the promoted conversation no longer holds the window', () => {
        const { session, source } = completeConversation(8)
        const chat = windowFrom(source, 5)
        const windowed = staleable(chat, 5)
        const controller = createHistoryWindowController({
            captureWindowed: () => null,
            captureSession: () => source,
            getCurrentSession: () => session,
            readLiveMetadata: () => cloneConversationMetadata(source.conversation),
        }, chat, 5, windowed.controller)

        session.delete(session.locate(6))
        windowed.expire()

        expect(controller.isCurrent()).toBe(false)
        expect(controller.applyRange(0, 0, [message('reply')], 'append')).toBe(false)
        expect(session.totalMessages).toBe(7)
    })

    it('recaptures a windowed backend when the conversation is still windowed', () => {
        const chat: Chat = { id: 'conversation-a', name: '', note: '', localLore: [], message: [message('a')] }
        const first = staleable(chat, 3)
        const second = staleable(chat, 3)
        const captureWindowed = vi.fn(() => second.controller)
        const controller = createHistoryWindowController({
            captureWindowed,
            captureSession: () => null,
            getCurrentSession: () => null,
            readLiveMetadata: () => ({ id: 'conversation-a', name: '', note: '', localLore: [] }),
        }, chat, 3, first.controller)

        first.expire()
        expect(controller.applyRange(1, 0, [message('b')], 'append')).toBe(true)
        expect(captureWindowed).toHaveBeenCalledWith(chat, 3)
        expect(second.applyRange).toHaveBeenCalledTimes(1)
    })

    it('stops writing after release', () => {
        const chat: Chat = { id: 'conversation-a', name: '', note: '', localLore: [], message: [] }
        const backend = staleable(chat, 0)
        const controller = createHistoryWindowController({
            captureWindowed: () => backend.controller,
            captureSession: () => null,
            getCurrentSession: () => null,
            readLiveMetadata: () => ({ id: 'conversation-a', name: '', note: '', localLore: [] }),
        }, chat, 0, backend.controller)

        controller.release()
        expect(backend.controller.release).toHaveBeenCalled()
        expect(controller.isCurrent()).toBe(false)
        expect(controller.applyRange(0, 0, [message('a')], 'append')).toBe(false)
    })
})

describe('history window metadata', () => {
    function metadataHarness() {
        const { session, source } = completeConversation(4)
        const chat = windowFrom(source, 2)
        const controller = createHistoryWindowController({
            captureWindowed: () => null,
            captureSession: () => source,
            getCurrentSession: () => session,
            readLiveMetadata: () => cloneConversationMetadata(source.conversation),
        }, chat, 2)
        return { session, source, chat, controller }
    }

    it('keeps chat variables set on the live conversation and on the window', () => {
        const { source, chat, controller } = metadataHarness()
        source.conversation.scriptstate = { $fromParser: 'live' }
        chat.scriptstate = { $fromTrigger: 'window' }

        expect(controller.applyRange(2, 0, [message('reply')], 'append')).toBe(true)

        expect(source.conversation.scriptstate).toEqual({ $fromParser: 'live', $fromTrigger: 'window' })
        expect(chat.scriptstate).toEqual({ $fromParser: 'live', $fromTrigger: 'window' })
    })

    it('lets the window win a conflicting change and keeps live-only fields', () => {
        const { source, chat, controller } = metadataHarness()
        source.conversation.scriptstate = { $mood: 'live' }
        source.conversation.note = 'live note'
        chat.scriptstate = { $mood: 'window' }

        expect(controller.applyRange(0, 0, [], 'update-metadata')).toBe(true)

        expect(source.conversation.scriptstate).toEqual({ $mood: 'window' })
        expect(source.conversation.note).toBe('live note')
    })

    it('pulls live changes into the window without writing when the window has none', () => {
        const { session, source, chat, controller } = metadataHarness()
        source.conversation.scriptstate = { $fromParser: 'live' }
        const version = session.version

        expect(controller.reconcileMetadata()).toBe(true)
        expect(chat.scriptstate).toEqual({ $fromParser: 'live' })
        expect(session.version).toBe(version)

        chat.lastMemory = 'id-m3'
        expect(controller.reconcileMetadata()).toBe(true)
        expect(source.conversation.lastMemory).toBe('id-m3')
        expect(session.version).toBe(version + 1)
    })
})
