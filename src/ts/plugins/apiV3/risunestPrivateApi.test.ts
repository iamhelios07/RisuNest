import { describe, expect, it, vi } from 'vitest'
import type { ConversationContext } from '../conversationContext'
import { createRisunestPrivateApi } from './risunestPrivateApi'

function setup(granted: boolean | Promise<boolean> = true) {
    const context = { revision: 1 } as ConversationContext
    const readConversationContext = vi.fn(async () => context)
    const hasDatabasePermission = vi.fn(async () => granted)
    const lifetime = new AbortController()
    const api = createRisunestPrivateApi({
        databaseAccess: { readConversationContext },
        hasDatabasePermission,
        lifetimeSignal: lifetime.signal,
    })
    return { api, context, readConversationContext, hasDatabasePermission, lifetime }
}

describe('risunestReadConversationContext', () => {
    it('reads parts that need no permission without asking for one', async () => {
        const { api, context, readConversationContext, hasDatabasePermission } = setup()

        await expect(api.readConversationContext({
            characterId: 'char',
            conversationId: 'conv',
            include: { character: true, lore: true, groupMembers: true },
            chatVariables: ['$key'],
            messages: { limit: 4, extraFields: ['__plugin'] },
        })).resolves.toBe(context)

        expect(hasDatabasePermission).not.toHaveBeenCalled()
        expect(readConversationContext).toHaveBeenCalledWith({
            target: { characterId: 'char', conversationId: 'conv' },
            include: { character: true, lore: true, persona: false, groupMembers: true, globals: false },
            chatVariables: ['$key'],
            messages: { window: { limit: 4 }, extraFields: ['__plugin'] },
        }, { allowPrivate: false, signal: expect.any(AbortSignal) })
    })

    it.each([
        ['persona', true],
        ['persona', false],
        ['globals', true],
        ['globals', false],
    ])('passes the db permission decision for %s (granted: %s)', async (part, granted) => {
        const { api, readConversationContext, hasDatabasePermission } = setup(granted)

        await api.readConversationContext({ include: { [part]: true } })

        expect(hasDatabasePermission).toHaveBeenCalledTimes(1)
        expect(readConversationContext).toHaveBeenCalledWith(
            expect.objectContaining({ target: null }),
            expect.objectContaining({ allowPrivate: granted }),
        )
    })

    it.each([
        ['an invalid part', { include: { globals: true }, chatVariables: 5 }],
        ['a signal that is not an AbortSignal', { include: { persona: true }, signal: 'stop' }],
    ])('rejects %s before asking or reading', async (_name, input) => {
        const { api, readConversationContext, hasDatabasePermission } = setup()

        await expect(api.readConversationContext(input)).rejects.toThrow(TypeError)

        expect(hasDatabasePermission).not.toHaveBeenCalled()
        expect(readConversationContext).not.toHaveBeenCalled()
    })

    it('stops for an aborted caller signal or a plugin unload during the permission prompt', async () => {
        const aborted = setup()
        const controller = new AbortController()
        controller.abort()
        await expect(aborted.api.readConversationContext({ signal: controller.signal }))
            .rejects.toMatchObject({ name: 'AbortError' })
        expect(aborted.readConversationContext).not.toHaveBeenCalled()

        let answer!: (granted: boolean) => void
        const unloading = setup(new Promise<boolean>((resolve) => { answer = resolve }))
        const pending = unloading.api.readConversationContext({ include: { globals: true } })
        await vi.waitFor(() => expect(unloading.hasDatabasePermission).toHaveBeenCalled())
        unloading.lifetime.abort()
        answer(true)
        await expect(pending).rejects.toMatchObject({ name: 'AbortError' })
        expect(unloading.readConversationContext).not.toHaveBeenCalled()
    })
})
