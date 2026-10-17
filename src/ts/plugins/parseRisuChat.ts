import rfdc from 'rfdc'
import { get } from 'svelte/store'
import { DBState, selectedCharID } from '../stores.svelte'
import { risuChatParser, type CbsConditions } from '../parser/parser.svelte'
import { getChatVarFromConversation, setChatVarOnConversation } from '../parser/chatVar.svelte'
import { planChatParserHistory } from '../chatParserHistory'
import { createLiveChatParserIndirections } from '../liveDisplayParserLease'
import { captureResidentCbsHistory, prepareCbsHistory } from '../prepareCbsHistory'
import { getModuleLorebooks, getModuleRegexScripts, getModules } from '../process/modules'
import { findActiveHistoryWindow } from '../process/historyWindowIndex'
import { writeHistoryWindowMessage } from '../process/historyWindowWrite'
import { setConversationChatVariable } from '../process/selectedChatVariables'
import { requestScriptDisplayRefresh } from '../process/scriptDisplayRefresh'
import { getPersonaPrompt, getUserName } from '../util'
import { cloneConversationMetadata, isMetadataOnlySelectedConversation } from '../storage/selectedConversationLifecycle'
import { assertPersistentMutationAllowed, getPersistentStorageAuthorityEpoch, getPersistentDataRuntime, peekActiveConversationSession } from '../storage/persistentDataRuntime.svelte'
import type { Chat, Message } from '../storage/database.svelte'

const clone = rfdc()

export interface ParseRisuChatOptions {
    messageIndex?: number
    role?: string
    processRegex?: boolean
    runVar?: boolean
    rmVar?: boolean
    tokenizeAccurate?: boolean
    cbsConditions?: CbsConditions
}

export class PluginParserTargetChangedError extends Error {
    constructor() {
        super('The parseRisuChat conversation is no longer available')
        this.name = 'PluginParserTargetChangedError'
    }
}

export async function parseRisuChat(text: string, options?: ParseRisuChatOptions, signal?: AbortSignal): Promise<string> {
    signal?.throwIfAborted()
    // Capture before the first await. A navigation never retargets this invocation.
    const sourceDatabase = DBState.db
    const sourceCharacter = sourceDatabase.characters[get(selectedCharID)]
    if (!sourceCharacter) throw new Error('No character selected')
    const sourceChat = sourceCharacter.chats?.[sourceCharacter.chatPage]
    if (!sourceChat) throw new Error('No active chat found')
    const epoch = getPersistentStorageAuthorityEpoch()
    const characterId = sourceCharacter.chaId
    const conversationId = sourceChat.id
    const chat = { ...cloneConversationMetadata(sourceChat), message: [] } as Chat
    const character = clone({ ...sourceCharacter, chats: [chat], chatPage: 0 })
    const database = clone({ ...sourceDatabase, characters: [character] })
    database.characters[0] = character
    const invocationChat = character.chats[0]
    const personaPrompt = getPersonaPrompt()
    const userName = getUserName()
    const modules = clone(getModules())
    const moduleLorebooks = clone(getModuleLorebooks())
    const moduleRegexScripts = clone(getModuleRegexScripts())
    const plan = planChatParserHistory({
        source: text ?? '',
        indirections: createLiveChatParserIndirections(database, character, invocationChat, personaPrompt),
    })
    const preparedPlan = options?.processRegex ? { ...plan, requiresFullHistory: true } : plan
    const chatID = options?.messageIndex ?? -1
    const windowed = isMetadataOnlySelectedConversation(sourceChat)
    const runtime = windowed ? getPersistentDataRuntime() : null
    const authority = runtime?.captureSelectedConversationAuthority()
    const activeWindow = findActiveHistoryWindow(characterId, conversationId)
    const totalMessages = windowed
        ? authority?.totalMessages
        : sourceChat.message.length
    if (totalMessages === undefined) throw new PluginParserTargetChangedError()
    if (!Number.isInteger(chatID) || chatID < -1 || chatID >= totalMessages) {
        throw new Error(`Invalid messageIndex: ${chatID}`)
    }
    const pendingVariables = new Map<string, string>()
    const pendingMessages = new Map<number, { expected: Message; data: string }>()
    const matchesMessage = (actual: Message | undefined, expected: Message) => actual
        && actual.chatId === expected.chatId && (actual.chatId || actual.data === expected.data)
    let mutationError: unknown
    const residentChat = () => {
        assertPersistentMutationAllowed(epoch)
        return DBState.db.characters.find((candidate) => candidate.chaId === characterId)
            ?.chats?.find((candidate) => conversationId ? candidate.id === conversationId : candidate === sourceChat)
    }
    const setVariable = (key: string, value: string) => {
        try {
            signal?.throwIfAborted()
            const current = residentChat()
            const session = peekActiveConversationSession()
            const selected = DBState.db.characters[get(selectedCharID)]
            if (current && !isMetadataOnlySelectedConversation(current)
                && (session?.matchesConversation(characterId, current)
                    || (selected?.chaId === characterId && selected.chats[selected.chatPage] === current))) {
                setConversationChatVariable(characterId, current, key, value)
            } else pendingVariables.set(key, value)
            setChatVarOnConversation(invocationChat, key, value)
        } catch (error) {
            mutationError = error
            throw error
        }
    }
    async function commitPending() {
        if (!pendingVariables.size && !pendingMessages.size) return
        assertPersistentMutationAllowed(epoch)
        const variables = new Map(pendingVariables)
        const messages = new Map(pendingMessages)
        pendingVariables.clear()
        pendingMessages.clear()
        await getPersistentDataRuntime().commitPreparedUnitIntent('plugin-parse-risu-chat', async (reader) => {
            assertPersistentMutationAllowed(epoch)
            const metadata = await reader.readConversationMetadata(characterId, conversationId)
            if (!metadata) throw new PluginParserTargetChangedError()
            const scriptstate = { ...metadata.value.conversation.scriptstate }
            for (const [key, value] of variables) scriptstate['$' + key] = value
            const conversations = []
            for (const [index, { expected, data }] of messages) {
                const result = await reader.readConversationWindow({ characterId, conversationId, startIndex: index, limit: 1 })
                const message = result?.value.messages[0]
                if (!matchesMessage(message, expected)) {
                    throw new PluginParserTargetChangedError()
                }
                conversations.push({ type: 'replace-range' as const, characterId, conversationId,
                    start: index, deleteCount: 1, messages: [{ ...message, data }] })
            }
            return {
                unitMutations: variables.size ? [{ type: 'set' as const,
                    key: JSON.stringify(['conversation', characterId, conversationId, 'scriptstate']), value: scriptstate }] : [],
                conversations,
            }
        })
        const selected = DBState.db.characters[get(selectedCharID)]
        if (selected?.chaId === characterId && selected.chats[selected.chatPage]?.id === conversationId) {
            requestScriptDisplayRefresh({ keys: [...variables.keys()] })
        }
    }
    let historyAccess
    if (!windowed) {
        historyAccess = captureResidentCbsHistory(sourceChat.message, preparedPlan, chatID)
    } else {
        const resident = new Map<number, Message>()
        const needsRows = preparedPlan.requiresFullHistory || preparedPlan.absoluteMessageIndices.length > 0
            || preparedPlan.requirements.some((requirement) => requirement !== 'count')
        if (needsRows) {
            for (const row of runtime!.getActiveConversationViewportSource()?.captureResidentRows?.() ?? []) {
                resident.set(row.absoluteIndex, clone(row.message))
            }
            if (activeWindow) {
                activeWindow.chat.message.forEach((message, index) => resident.set(activeWindow.absoluteStartIndex + index, clone(message)))
            }
        }
        const lease = needsRows ? await runtime!.store.acquireRevision(authority!.storeRevision) : null
        try {
            historyAccess = await prepareCbsHistory({ characterId, conversationId, totalMessages, chatID,
                plan: preparedPlan, signal,
                reader: {
                    revision: authority!.storeRevision,
                    readResidentMessage: (index) => resident.get(index),
                    readRoleWindow: lease?.messageMetadataReadMode === 'projected' && authority!.sessionVersion === authority!.persistedSessionVersion
                        ? (query) => lease.readConversationMessageMetadataWindow!(query) : undefined,
                    readConversationWindow: async (query) => {
                        const result = await lease!.readConversationWindow(query)
                        // Resident stream rows account for the unpersisted suffix.
                        return result ? { ...result, value: { ...result.value, totalMessages } } : null
                    },
                },
            })
        } finally { await lease?.release() }
    }
    if (options?.processRegex) invocationChat.message = historyAccess.all() as Message[]
    const role = options?.role
    const globalChatVariables = { ...database.globalChatVariables }
    for (const [key, value] of Object.entries(invocationChat.GLGlobalVariables ?? {})) {
        if (value && value !== 'null') globalChatVariables[key] = value
    }
    const cbsConditions = { ...(role ? { chatRole: role } : {}), ...(options?.cbsConditions ?? {}) }
    const parser = {
        db: database, chara: character, chatID, role, cbsConditions,
        runVar: options?.runVar, rmVar: options?.rmVar, tokenizeAccurate: options?.tokenizeAccurate,
        historyAccess, selectedCharID: 0, selectedCharacterId: characterId,
        userName, personaPrompt, modules, moduleLorebooks,
        globalChatVariables,
        getChatVar: (key: string) => getChatVarFromConversation(database, characterId, invocationChat, key),
        setChatVar: setVariable,
    }
    try {
        signal?.throwIfAborted()
        const parsed = risuChatParser(text ?? '', { ...parser })
        historyAccess.assertComplete()
        if (mutationError) throw mutationError
        if (!options?.processRegex) return parsed
        await commitPending()
        const { processScriptFull } = await import('../process/scripts')
        const result = await processScriptFull(character, parsed, 'editprocess', chatID, cbsConditions, {
            signal,
            invocationContext: {
                database, character, chat: invocationChat, parser, moduleRegexScripts,
                inject(index, data) {
                    try {
                        signal?.throwIfAborted()
                        const expected = historyAccess.row(index)
                        if (!expected) throw new PluginParserTargetChangedError()
                        const current = residentChat()
                        const session = peekActiveConversationSession()
                        const selected = DBState.db.characters[get(selectedCharID)]
                        const window = findActiveHistoryWindow(characterId, conversationId)
                        if (current && session?.matchesConversation(characterId, current)) {
                            const locator = session.locate(index)
                            const message = session.readMessage(locator)
                            if (!matchesMessage(message, expected)) throw new PluginParserTargetChangedError()
                            session.edit(locator, { ...message, data })
                        } else if (current && window && index >= window.absoluteStartIndex) {
                            if (!matchesMessage(window.chat.message[index - window.absoluteStartIndex], expected)) throw new PluginParserTargetChangedError()
                            if (!writeHistoryWindowMessage(window, index, (message) => ({ ...message, data }))) throw new PluginParserTargetChangedError()
                        } else if (current && !isMetadataOnlySelectedConversation(current)
                            && selected?.chaId === characterId && selected.chats[selected.chatPage] === current) {
                            if (!matchesMessage(current.message[index], expected)) throw new PluginParserTargetChangedError()
                            current.message[index].data = data
                        } else pendingMessages.set(index, { expected: pendingMessages.get(index)?.expected ?? clone(expected), data })
                        expected.data = data
                    } catch (error) {
                        mutationError = error
                        throw error
                    }
                },
            },
        })
        if (mutationError) throw mutationError
        return result.data
    } finally { await commitPending() }
}
