import {
    classifyChatParserHistory,
    planChatParserHistory,
    type ChatParserHistoryClassificationInput,
} from './chatParserHistory'
import { collectLiveChatParserUnsafeDependencies } from './selectedConversationLiveParserProjection'
import {
    getCurrentCharacter,
    getCurrentChat,
    getDatabase,
    type character,
    type Database,
    type Chat,
} from './storage/database.svelte'
import type { simpleCharacterArgument } from './parser/parser.svelte'
import { getPersistentDataRuntime } from './storage/persistentDataRuntime.svelte'
import type { PersistentDataRuntime } from './storage/persistentDataRuntime'
import { getModuleAssets, getModuleLorebooks, getModuleRegexScripts, getModules, getModuleTriggers } from './process/modules'
import { pluginV2 } from './plugins/plugins.svelte'
import { findCharacterbyId, getPersonaPrompt, getUserName, parseKeyValue } from './util'
import { createChatParserDependencyStamp } from './chatRenderIdentity'
import rfdc from 'rfdc'
import { cloneConversationMetadata, isMetadataOnlySelectedConversation } from './storage/selectedConversationLifecycle'
import { captureResidentCbsHistory, prepareCbsHistory } from './prepareCbsHistory'
import type { BoundedLiveChatParserProjection } from './selectedConversationLiveParserProjection'

export interface LiveDisplayParserLease {
    projection?: BoundedLiveChatParserProjection
    release(): void
}

type DisplayCharacter = character | simpleCharacterArgument | string | null
type DisplayRuntime = Pick<
    PersistentDataRuntime,
    'captureSelectedConversationTarget' | 'acquireCompleteConversation'
>
interface LiveDisplayParserInputs extends ChatParserHistoryClassificationInput {
    requiresCompleteConversation?: boolean
    triggerStamp?: string | null
    variableStamp?: string | null
}

export function createLiveChatParserIndirections(
    database: Database,
    selectedCharacter: character,
    conversation: Chat,
    personaPrompt: string,
): Readonly<Record<string, unknown>> {
    const character = selectedCharacter
    let authorNote = conversation.note ?? ''
    if (!authorNote) {
        for (const item of database.promptTemplate ?? []) {
            if (item.type !== 'authornote' || !item.defaultText) continue
            authorNote = item.defaultText
            break
        }
    }
    return {
        personality: character?.personality ?? '',
        charpersona: character?.personality ?? '',
        description: character?.desc ?? '',
        chardesc: character?.desc ?? '',
        scenario: character?.scenario ?? '',
        exampledialogue: character?.exampleMessage ?? '',
        examplemessage: character?.exampleMessage ?? '',
        persona: personaPrompt,
        userpersona: personaPrompt,
        mainprompt: database.mainPrompt ?? '',
        systemprompt: database.mainPrompt ?? '',
        jb: database.jailbreak ?? '',
        jailbreak: database.jailbreak ?? '',
        globalnote: database.globalNote ?? '',
        systemnote: database.globalNote ?? '',
        ujb: database.globalNote ?? '',
        authornote: authorNote,
    }
}

export function createLiveChatParserSource(
    database: Database,
    character: Exclude<DisplayCharacter, string | null>,
    moduleRegex: ReturnType<typeof getModuleRegexScripts>,
) {
    return {
        guiHTML: database.theme === 'customHTML' ? database.guiHTML : '',
        presetRegex: database.presetRegex ?? [],
        characterRegex: character.customscript ?? [],
        moduleRegex,
    }
}

export function captureLiveDisplayParserInputs(
    source: unknown,
    renderCharacter: DisplayCharacter,
): LiveDisplayParserInputs {
    const database = getDatabase()
    const selectedCharacter = getCurrentCharacter()
    const conversation = getCurrentChat()
    const character =
        typeof renderCharacter === 'string'
            ? findCharacterbyId(renderCharacter)
            : (renderCharacter ?? selectedCharacter)
    if (!selectedCharacter || !conversation || !character) return { source }
    const moduleRegex = getModuleRegexScripts()
    const moduleTriggers = getModuleTriggers()
    const triggers =
        [...(character.triggerscript ?? []), ...moduleTriggers]
    return {
        source: [source, createLiveChatParserSource(database, character, moduleRegex)],
        indirections: createLiveChatParserIndirections(
            database,
            selectedCharacter,
            conversation,
            getPersonaPrompt(),
        ),
        // Keep live script edits reactive without treating script source as CBS
        // history, or depending on lowLevelAccess which Lua execution resets.
        triggerStamp: createChatParserDependencyStamp({
            chaId: character.chaId,
            triggerscript: triggers.map(({ comment, type, conditions, effect }) => ({
                comment,
                type,
                conditions,
                effect,
            })),
        }),
        variableStamp: JSON.stringify({
            scriptstate: conversation.scriptstate,
            globalChatVariables: database.globalChatVariables,
            GLGlobalVariables: conversation.GLGlobalVariables,
            defaultVariables: selectedCharacter.defaultVariables,
            templateDefaultVariables: database.templateDefaultVariables,
            userName: getUserName(),
        }),
        unsafeDependencies: collectLiveChatParserUnsafeDependencies({
            triggers,
            pluginV2EditDisplay: pluginV2.editdisplay.size > 0,
            regexScripts: [
                ...(database.presetRegex ?? []),
                ...(character.customscript ?? []),
                ...moduleRegex,
            ],
        }),
    }
}

export function createLiveDisplayParserLeaseAcquirer(dependencies: {
    runtime(): DisplayRuntime
    classify(source: unknown, character: DisplayCharacter): LiveDisplayParserInputs
    prepare?(inputs: LiveDisplayParserInputs, chatID: number, signal: AbortSignal, firstPass?: string): Promise<BoundedLiveChatParserProjection>
}) {
    return async (request: {
        source: unknown
        character: DisplayCharacter
        signal: AbortSignal
        chatID?: number
        parseFirst?: boolean
    }): Promise<LiveDisplayParserLease | null> => {
        request.signal.throwIfAborted()
        const inputs = dependencies.classify(request.source, request.character)
        if (dependencies.prepare && !inputs.requiresCompleteConversation && !inputs.unsafeDependencies?.length) {
            return { projection: await dependencies.prepare(inputs, request.chatID ?? -1, request.signal,
                request.parseFirst && typeof request.source === 'string' ? request.source : undefined), release() {} }
        }
        const classification = classifyChatParserHistory(inputs)
        if (
            !inputs.requiresCompleteConversation &&
            !classification.requiresFullHistory &&
            classification.absoluteMessageIndices.length === 0
        )
            return null
        const runtime = dependencies.runtime()
        const target = runtime.captureSelectedConversationTarget()
        if (!target) throw new Error('Live display has no selected conversation')
        const lease = await runtime.acquireCompleteConversation('live-display-parser', target)
        let released = false
        const release = () => {
            if (released) return
            released = true
            lease.release()
        }
        try {
            request.signal.throwIfAborted()
            const current = runtime.captureSelectedConversationTarget()
            if (
                !current ||
                current.characterId !== target.characterId ||
                current.conversationId !== target.conversationId ||
                current.navigationGeneration !== target.navigationGeneration ||
                lease.target.characterId !== target.characterId ||
                lease.target.conversationId !== target.conversationId ||
                lease.target.navigationGeneration !== target.navigationGeneration ||
                current.storeRevision !== lease.target.storeRevision
            ) {
                throw new Error('Live display conversation changed during preparation')
            }
            return { release }
        } catch (error) {
            release()
            throw error
        }
    }
}

export const acquireLiveDisplayParserLease = createLiveDisplayParserLeaseAcquirer({
    runtime: () => getPersistentDataRuntime(),
    classify: captureLiveDisplayParserInputs,
    prepare: prepareLiveDisplayParserProjection,
})

async function prepareLiveDisplayParserProjection(inputs: LiveDisplayParserInputs, chatID: number, signal: AbortSignal, firstPass?: string): Promise<BoundedLiveChatParserProjection> {
    const clone = rfdc()
    const runtime = getPersistentDataRuntime()
    const target = runtime.captureSelectedConversationTarget()
    const original = getCurrentCharacter()
    const originalChat = getCurrentChat()
    if (!target || !original || !originalChat) throw new Error('Live display has no selected conversation')
    const character = clone({ ...original, chats: [{ ...cloneConversationMetadata(originalChat), message: [] }], chatPage: 0 })
    const database = clone({ ...getDatabase(), characters: [character] })
    database.characters[0] = character
    const chatVariables = Object.fromEntries([
        ...parseKeyValue(database.templateDefaultVariables ?? ''),
        ...parseKeyValue(character.defaultVariables ?? ''),
    ])
    for (const [key, value] of Object.entries(originalChat.scriptstate ?? {})) chatVariables[key.replace(/^\$/, '')] = String(value)
    const globalChatVariables = { ...database.globalChatVariables }
    for (const [key, value] of Object.entries(originalChat.GLGlobalVariables ?? {})) {
        if (value && value !== 'null') globalChatVariables[key] = value
    }
    const context = {
        presetRegex: database.presetRegex ?? [], moduleRegexScripts: clone(getModuleRegexScripts()),
        moduleAssets: clone(getModuleAssets()), dynamicAssets: database.dynamicAssets,
        dynamicAssetsEditDisplay: database.dynamicAssetsEditDisplay,
        parserContext: { database, character, selectedCharID: 0, chatVariables, globalChatVariables,
            userName: getUserName(), personaPrompt: getPersonaPrompt(), modules: clone(getModules()),
            moduleLorebooks: clone(getModuleLorebooks()), currentTime: Date.now() },
    }
    let plan = planChatParserHistory(inputs)
    const authority = runtime.captureSelectedConversationAuthority()
    const source = runtime.getActiveConversationViewportSource()
    const version = source?.snapshot().version
    const historyVersion = source?.snapshot().historyVersion
    const assertCurrent = () => {
        signal.throwIfAborted()
        const current = runtime.captureSelectedConversationTarget()
        if (!current || current.characterId !== target.characterId || current.conversationId !== target.conversationId
            || current.navigationGeneration !== target.navigationGeneration || source?.snapshot().version !== version
            || runtime.getActiveConversationViewportSource() !== source || source?.snapshot().historyVersion !== historyVersion) {
            throw new Error('Live display conversation changed during preparation')
        }
    }
    const totalMessages = isMetadataOnlySelectedConversation(originalChat) ? authority!.totalMessages : originalChat.message.length
    let historyAccess
    let parsedSource: string | undefined
    const parseFirst = async () => {
        if (firstPass === undefined) return
        const { risuChatParser } = await import('./parser/parser.svelte')
        parsedSource = risuChatParser(firstPass, { ...context.parserContext, db: database, chara: character, chatID, historyAccess })
        historyAccess.assertComplete()
        plan = planChatParserHistory({ ...inputs, source: [inputs.source, parsedSource] })
    }
    if (!isMetadataOnlySelectedConversation(originalChat)) {
        historyAccess = captureResidentCbsHistory(originalChat.message, plan, chatID)
        if (firstPass !== undefined) {
            await parseFirst()
            assertCurrent()
            historyAccess = captureResidentCbsHistory(originalChat.message, plan, chatID)
        }
    } else {
        const resident = new Map((source?.captureResidentRows?.() ?? []).map((row) => [row.absoluteIndex, clone(row.message)]))
        const lease = await runtime.store.acquireRevision(authority!.storeRevision)
        try {
            const prepare = () => prepareCbsHistory({ characterId: target.characterId, conversationId: target.conversationId,
                totalMessages, chatID, plan, signal, assertCurrent, reader: {
                    revision: lease.revision, readResidentMessage: (index) => resident.get(index),
                    readRoleWindow: lease.messageMetadataReadMode === 'projected' && authority!.sessionVersion === authority!.persistedSessionVersion
                        ? (query) => lease.readConversationMessageMetadataWindow!(query) : undefined,
                    readConversationWindow: async (query) => {
                        const result = await lease.readConversationWindow(query)
                        return result ? { ...result, value: { ...result.value, totalMessages } } : null
                    },
                } })
            historyAccess = await prepare()
            if (firstPass !== undefined) {
                await parseFirst()
                historyAccess = await prepare()
            }
        } finally { await lease.release() }
    }
    assertCurrent()
    const prepareHistory = async (text: string, index: number, complete: boolean) => {
        assertCurrent()
        const plan = planChatParserHistory({ source: text, indirections: inputs.indirections,
            unsafeDependencies: complete ? ['regex-reparse'] : [] })
        if (!isMetadataOnlySelectedConversation(originalChat)) return captureResidentCbsHistory(originalChat.message, plan, index)
        const resident = new Map((source?.captureResidentRows?.() ?? []).map((row) => [row.absoluteIndex, clone(row.message)]))
        const lease = await runtime.store.acquireRevision(authority!.storeRevision)
        try {
            return await prepareCbsHistory({ characterId: target.characterId, conversationId: target.conversationId,
                totalMessages, chatID: index, plan, signal, assertCurrent, reader: {
                    revision: lease.revision, readResidentMessage: (index) => resident.get(index),
                    readRoleWindow: lease.messageMetadataReadMode === 'projected' && authority!.sessionVersion === authority!.persistedSessionVersion
                        ? (query) => lease.readConversationMessageMetadataWindow!(query) : undefined,
                    readConversationWindow: async (query) => {
                        const result = await lease.readConversationWindow(query)
                        return result ? { ...result, value: { ...result.value, totalMessages } } : null
                    },
                } })
        } finally { await lease.release() }
    }
    return { kind: 'bounded', characterId: target.characterId, conversationId: target.conversationId,
        revision: target.storeRevision, totalMessages, chatID, projectedChatID: chatID, historyOffset: 0, messages: [],
        parsedSource, context: { ...context, parserContext: { ...context.parserContext, historyAccess, prepareHistory } } }
}

export function captureLiveDisplayParserSelection(): string {
    const target = getPersistentDataRuntime().captureSelectedConversationTarget()
    return target
        ? JSON.stringify([target.characterId, target.conversationId, target.navigationGeneration])
        : ''
}

export function subscribeLiveDisplayParserSelection(
    listener: (identity: string, historyRevision?: string) => void,
): () => void {
    const runtime = getPersistentDataRuntime()
    let unsubscribeSource: (() => void) | undefined
    const update = () => {
        const source = runtime.getActiveConversationViewportSource()
        const snapshot = source?.snapshot()
        listener(captureLiveDisplayParserSelection(), JSON.stringify([snapshot?.sourceToken, snapshot?.version, snapshot?.historyVersion]))
    }
    const bindSource = () => {
        unsubscribeSource?.()
        unsubscribeSource = runtime.getActiveConversationViewportSource()?.subscribe(update)
        update()
    }
    const unsubscribe = runtime.subscribeActiveConversationViewportSource(bindSource)
    bindSource()
    return () => { unsubscribe(); unsubscribeSource?.() }
}
