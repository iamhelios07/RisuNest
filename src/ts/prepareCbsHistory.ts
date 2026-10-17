import rfdc from 'rfdc'
import { createCbsHistoryAccess, cbsHistorySearchKey, type CbsHistoryAccess } from './cbsHistoryAccess'
import type { CbsHistoryPlan } from './chatParserHistory'
import type { Message } from './storage/database.svelte'
import type { PersistentRevisionReader } from './storage/persistentDataStore'

const clone = rfdc()

function requests(plan: CbsHistoryPlan, chatID: number, total: number) {
    const indices = new Set(plan.absoluteMessageIndices)
    const searches: { role: Message['role']; before: number; count: number }[] = []
    for (const requirement of plan.requirements) {
        switch (requirement) {
            case 'current': if (chatID !== -1) indices.add(chatID); break
            case 'tail': indices.add(total - 1); break
            case 'previouschar': searches.push({ role: 'char', before: chatID === -1 ? total : chatID, count: 1 }); break
            case 'previoususer': if (chatID !== -1) searches.push({ role: 'user', before: chatID, count: 1 }); break
            case 'idle': if (chatID !== -1) searches.push({ role: 'user', before: chatID + 1, count: 2 }); break
        }
    }
    return { indices: [...indices].filter((index) => index >= 0 && index < total), searches }
}

export function captureResidentCbsHistory(messages: readonly Message[], plan: CbsHistoryPlan, chatID: number): CbsHistoryAccess {
    const rows = new Map<number, Message>()
    if (plan.requiresFullHistory) return createCbsHistoryAccess({ length: messages.length, rows, complete: clone(messages) })
    const needed = requests(plan, chatID, messages.length)
    const searches = new Map<string, readonly Message[]>()
    for (const index of needed.indices) rows.set(index, clone(messages[index]))
    for (const { role, before, count } of needed.searches) {
        const found: Message[] = []
        for (let index = before - 1; index >= 0 && found.length < count; index--) {
            if (messages[index].role !== role) continue
            if (!rows.has(index)) rows.set(index, clone(messages[index]))
            found.push(rows.get(index)!)
        }
        searches.set(cbsHistorySearchKey(role, before, count), found)
    }
    return createCbsHistoryAccess({ length: messages.length, rows, searches })
}

export interface CbsHistoryReader {
    readonly revision: number
    readConversationWindow: PersistentRevisionReader['readConversationWindow']
    readResidentMessage?(index: number): Readonly<Message> | undefined
    /** Only supply this when the backend can avoid materializing message bodies. */
    readRoleWindow?: PersistentRevisionReader['readConversationMessageMetadataWindow']
}

export async function prepareCbsHistory(input: {
    characterId: string
    conversationId: string
    totalMessages: number
    chatID: number
    chatIDs?: readonly number[]
    plan: CbsHistoryPlan
    reader: CbsHistoryReader
    signal?: AbortSignal
    assertCurrent?: () => void
    pageSize?: number
}): Promise<CbsHistoryAccess> {
    const { reader, totalMessages, chatID, plan } = input
    let lastPage: { start: number; end: number; messages: Message[] } | undefined
    const pageSize = Math.min(128, Math.max(1, input.pageSize ?? 32))
    const rows = new Map<number, Message>()
    const searches = new Map<string, readonly Message[]>()
    function check() {
        input.signal?.throwIfAborted()
        input.assertCurrent?.()
    }
    function query(startIndex: number, limit: number) {
        return { characterId: input.characterId, conversationId: input.conversationId, startIndex, limit }
    }
    function validate(result: Awaited<ReturnType<CbsHistoryReader['readConversationWindow']>> | Awaited<ReturnType<NonNullable<CbsHistoryReader['readRoleWindow']>>>, start: number, end: number) {
        check()
        if (!result || result.revision !== reader.revision || result.value.characterId !== input.characterId
            || result.value.conversationId !== input.conversationId || result.value.startIndex !== start
            || result.value.endIndex !== end || result.value.messages.length !== end - start
            || result.value.totalMessages !== totalMessages) {
            throw new Error('CBS history reader returned a different conversation range')
        }
        for (let index = 0; index < end - start; index++) {
            if (!result.value.messages[index] || typeof result.value.messages[index] !== 'object') {
                throw new Error('CBS history conversation window contains an invalid message row')
            }
        }
    }
    async function page(start: number, end: number): Promise<Message[]> {
        check()
        if (lastPage?.start === start && lastPage.end === end) return lastPage.messages
        const result: Message[] = []
        for (let index = start; index < end;) {
            const resident = rows.get(index) ?? reader.readResidentMessage?.(index)
            if (resident) {
                result.push(clone(resident))
                index++
                continue
            }
            let next = index + 1
            while (next < end && !rows.has(next) && !reader.readResidentMessage?.(next)) next++
            const window = await reader.readConversationWindow(query(index, next - index))
            validate(window, index, next)
            result.push(...window!.value.messages.map(clone))
            index = next
        }
        check()
        lastPage = { start, end, messages: result }
        return result
    }
    async function row(index: number) {
        if (index < 0 || index >= totalMessages || rows.has(index)) return
        rows.set(index, (await page(index, index + 1))[0])
    }
    async function previous(role: Message['role'], before: number, count: number) {
        const found: Message[] = []
        for (let end = Math.min(before, totalMessages); end > 0 && found.length < count;) {
            while (end > 0 && found.length < count) {
                const resident = rows.get(end - 1) ?? reader.readResidentMessage?.(end - 1)
                if (!resident) break
                end--
                if (resident.role === role) {
                    const message = clone(resident)
                    rows.set(end, message)
                    found.push(message)
                }
            }
            if (end === 0 || found.length === count) break
            const start = Math.max(0, end - pageSize)
            // A resident page wins over metadata I/O, including unpersisted stream rows.
            const allResident = Array.from({ length: end - start }, (_, i) =>
                rows.get(start + i) ?? reader.readResidentMessage?.(start + i)).every(Boolean)
            if (reader.readRoleWindow && !allResident) {
                const metadata = await reader.readRoleWindow({ ...query(start, end - start), skipParserWork: true })
                validate(metadata, start, end)
                for (let index = end - 1; index >= start && found.length < count; index--) {
                    const resident = rows.get(index) ?? reader.readResidentMessage?.(index)
                    if ((resident?.role ?? metadata!.value.messages[index - start].role) !== role) continue
                    await row(index)
                    found.push(rows.get(index)!)
                }
            } else {
                const messages = await page(start, end)
                for (let index = end - 1; index >= start && found.length < count; index--) {
                    if (messages[index - start].role !== role) continue
                    const message = messages[index - start]
                    rows.set(index, message)
                    found.push(message)
                }
            }
            end = start
        }
        searches.set(cbsHistorySearchKey(role, before, count), found)
    }
    check()
    if (plan.requiresFullHistory) {
        const complete: Message[] = []
        for (let start = 0; start < totalMessages; start += pageSize) {
            complete.push(...await page(start, Math.min(totalMessages, start + pageSize)))
        }
        return createCbsHistoryAccess({ length: totalMessages, rows, complete })
    }
    for (const current of input.chatIDs ?? [chatID]) {
        const needed = requests(plan, current, totalMessages)
        for (const index of needed.indices) await row(index)
        for (const { role, before, count } of needed.searches) {
            if (!searches.has(cbsHistorySearchKey(role, before, count))) await previous(role, before, count)
        }
    }
    check()
    return createCbsHistoryAccess({ length: totalMessages, rows, searches })
}
