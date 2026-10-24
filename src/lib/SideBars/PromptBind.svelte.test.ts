import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { mount, tick, unmount } from 'svelte'
import { get } from 'svelte/store'
import { languageEnglish } from 'src/lang/en'
import { BotSettingsSubMenu, DBState, selectedCharID, openPresetList, settingsOpen, SettingsMenuIndex } from 'src/ts/stores.svelte'
import { getDatabase, normalizeDatabaseDefaults, setDatabase, type Database } from 'src/ts/storage/database.svelte'
import { doingChat } from 'src/ts/process/generationState'
import PromptBind from './PromptBind.svelte'
import { createMetadataOnlySelectedConversation } from 'src/ts/storage/selectedConversationLifecycle'

const storage = vi.hoisted(() => ({ metadata: {} as Record<string, unknown>, navigation: 0 }))
vi.mock('src/ts/parser/parser.svelte', async () => (await import('src/ts/parser/parser.testSupport')).parserStubModule())
vi.mock('src/ts/storage/persistentDataRuntime.svelte', async (importOriginal) => ({
    ...await importOriginal<typeof import('src/ts/storage/persistentDataRuntime.svelte')>(),
    getPersistentNavigationGeneration: () => storage.navigation,
    getPersistentStorageAuthorityEpoch: () => 0,
    getActiveConversationSession: () => null,
    flushPendingData: async () => {},
    isLoadedPreset: (preset: unknown) => typeof preset === 'object' && preset !== null && Object.hasOwn(preset, 'mainPrompt'),
    getPersistentDataRuntime: () => ({
        mutateConversationBinding: async (_owner: string, _chat: string, patch: Record<string, unknown>, publish: (patch: Record<string, unknown>) => void) => {
            storage.metadata = structuredClone(patch)
            publish(patch)
        },
    }),
}))

let instance: ReturnType<typeof mount>
const chat = () => DBState.db.characters[0].chats[0]
const trigger = () => document.querySelector<HTMLButtonElement>(`button[title="${languageEnglish.promptBinding}"]`)!
const option = (text: string) => [...document.querySelectorAll<HTMLButtonElement>('[role="dialog"] button')].find(button => button.textContent?.includes(text))!

beforeEach(async () => {
    storage.metadata = {}
    storage.navigation = 0
    doingChat.set(false)
    BotSettingsSubMenu.set(0)
    SettingsMenuIndex.set(-1)
    settingsOpen.set(false)
    openPresetList.set(false)
    const db = normalizeDatabaseDefaults({ language: 'en' } as Database)
    Object.assign(db.botPresets[0], { id: 'global', name: 'Global prompt', mainPrompt: 'Global text', aiModel: 'model-a', temperature: 40 })
    db.botPresets.push({ ...structuredClone(db.botPresets[0]), id: 'bound', name: 'Writing prompt', mainPrompt: 'Writing text', aiModel: 'model-b', temperature: 120 })
    db.mainPrompt = 'Global text'
    db.aiModel = 'model-a'
    db.temperature = 40
    db.characters = [{ chaId: 'owner', name: 'Owner', type: 'character', chatPage: 0, chats: [createMetadataOnlySelectedConversation({ id: 'chat', name: 'Chat', note: '', localLore: [] })] }] as unknown as Database['characters']
    setDatabase(db)
    selectedCharID.set(0)
    instance = mount(PromptBind, { target: document.body })
    await tick()
})

afterEach(async () => {
    await unmount(instance)
    doingChat.set(false)
    BotSettingsSubMenu.set(0)
    SettingsMenuIndex.set(-1)
    settingsOpen.set(false)
    openPresetList.set(false)
    document.body.replaceChildren()
})

it('opens preset management over the chatbot other settings without changing the binding or preset', async () => {
    chat().bindedBotPreset = 'bound'
    trigger().click()
    await tick()
    option(languageEnglish.presetManage).click()
    await vi.waitFor(() => expect(get(openPresetList)).toBe(true))
    expect(document.querySelector('[role="dialog"]')).toBeNull()
    expect(get(settingsOpen)).toBe(true)
    expect(get(SettingsMenuIndex)).toBe(1)
    expect(get(BotSettingsSubMenu)).toBe(3)
    expect(chat().bindedBotPreset).toBe('bound')
    expect(getDatabase().botPresetsId).toBe(0)
    expect(storage.metadata).toEqual({})
})

it('searches and binds the complete prompt preset, then restores the global model on unbind', async () => {
    trigger().click()
    await tick()
    const search = document.querySelector<HTMLInputElement>('[role="dialog"] input')!
    search.value = 'WRITING'
    search.dispatchEvent(new Event('input', { bubbles: true }))
    await tick()
    expect(option('Writing prompt')).toBeDefined()
    option('Writing prompt').click()
    await vi.waitFor(() => expect(document.querySelector('[role="dialog"]')).toBeNull())
    expect(storage.metadata).toEqual({ bindedBotPreset: 'bound' })
    expect(() => chat().message).toThrow('metadata-only')
    expect(chat().bindedBotPreset).toBe('bound')
    expect([getDatabase().mainPrompt, getDatabase().aiModel, getDatabase().temperature]).toEqual(['Writing text', 'model-b', 120])
    expect(getDatabase().botPresetsId).toBe(0)

    trigger().click()
    await tick()
    option(languageEnglish.inheritPromptPreset).click()
    await vi.waitFor(() => expect(document.querySelector('[role="dialog"]')).toBeNull())
    expect(chat().bindedBotPreset).toBe('')
    expect([getDatabase().mainPrompt, getDatabase().aiModel, getDatabase().temperature]).toEqual(['Global text', 'model-a', 40])
})

it('rejects a selection captured before navigation and blocks opening during generation', async () => {
    doingChat.set(true)
    trigger().click()
    await tick()
    expect(document.querySelector('[role="dialog"]')).toBeNull()
    doingChat.set(false)
    trigger().click()
    await tick()
    storage.navigation++
    option('Writing prompt').click()
    await tick()
    expect(storage.metadata).toEqual({})
    expect(chat().bindedBotPreset).toBeUndefined()
})

it('shows a missing binding explicitly and lets the user remove it', async () => {
    chat().bindedBotPreset = 'missing'
    await tick()
    expect(trigger().textContent).toContain(languageEnglish.missingBoundPromptPreset)
    trigger().click()
    await tick()
    option(languageEnglish.inheritPromptPreset).click()
    await vi.waitFor(() => expect(chat().bindedBotPreset).toBe(''))
})


it('restores the appropriate model when moving between a bound chat and a chat using the global default', async () => {
    const db = getDatabase()
    chat().bindedBotPreset = 'bound'
    db.characters[0].chats.push(createMetadataOnlySelectedConversation({ id: 'other-chat', name: 'Other', note: '', localLore: [] }))
    const { default: ChatBindingLifecycle } = await import('./ChatBindingLifecycle.svelte')
    const lifecycle = mount(ChatBindingLifecycle, { target: document.body })
    try {
        await vi.waitFor(() => expect(getDatabase().aiModel).toBe('model-b'))
        db.characters[0].chatPage = 1
        await vi.waitFor(() => expect(getDatabase().aiModel).toBe('model-a'))
        expect(getDatabase().mainPrompt).toBe('Global text')
        db.characters[0].chatPage = 0
        await vi.waitFor(() => expect(getDatabase().aiModel).toBe('model-b'))
        expect(getDatabase().botPresetsId).toBe(0)
    } finally {
        await unmount(lifecycle)
    }
})
