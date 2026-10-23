import { beforeEach, describe, expect, it, vi } from 'vitest'

const runtime = vi.hoisted(() => ({
    flushPendingData: vi.fn(async (_reason: string) => undefined),
    readPreset: vi.fn(async (_id: string): Promise<{ revision: number, value: unknown } | null> => null),
}))

vi.mock('../parser/parser.svelte', async () => (await import('../parser/parser.testSupport')).parserStubModule())
vi.mock('./persistentDataRuntime.svelte', async (importOriginal) => ({
    ...await importOriginal<typeof import('./persistentDataRuntime.svelte')>(),
    getPersistentNavigationGeneration: () => 0,
    getPersistentStorageAuthorityEpoch: () => 0,
    flushPendingData: runtime.flushPendingData,
    getPersistentDataRuntime: () => ({ store: { readPreset: runtime.readPreset } }),
}))

import { selectedCharID } from '../stores.svelte'
import type { Database, botPreset } from './database.svelte'
import {
    activatePresetOverride,
    activateChatPresetBinding,
    getDatabase,
    getEffectivePresetId,
    normalizeDatabaseDefaults,
    setDatabase,
    setEffectivePresetOverride,
} from './database.svelte'
import { isLoadedPreset, createProductionStateAdapter } from './persistentDataRuntime.svelte'

function presetDatabase(): Database {
    const database = normalizeDatabaseDefaults({} as Database)
    database.botPresets[0].id = 'preset-a'
    database.botPresets[0].mainPrompt = 'Prompt A'
    database.botPresets.push({ ...structuredClone(database.botPresets[0]), id: 'preset-b', name: 'Preset B', mainPrompt: 'Prompt B' })
    database.botPresetsId = 0
    database.mainPrompt = 'Prompt A'
    database.personas[0].id = 'persona'
    return database
}

let stored: Map<string, botPreset>

function deferred() {
    let resolve!: () => void
    const promise = new Promise<void>((done) => { resolve = done })
    return { promise, resolve }
}

beforeEach(() => {
    const database = presetDatabase()
    stored = new Map(database.botPresets.map((preset) => [preset.id!, structuredClone(preset)]))
    setDatabase(database)
    selectedCharID.set(-1)
    runtime.flushPendingData.mockReset().mockResolvedValue(undefined)
    runtime.readPreset.mockReset().mockImplementation(async (id) => {
        const value = stored.get(id)
        return value ? { revision: 1, value: structuredClone(value) } : null
    })
})

describe('preset chain override activation', () => {
    it('activates the preset the index named when the library changes during the flush', async () => {
        const added = { ...structuredClone(stored.get('preset-a')!), id: 'preset-c', name: 'Preset C', mainPrompt: 'Prompt C' }
        stored.set(added.id, added)
        runtime.flushPendingData.mockImplementation(async () => {
            const database = getDatabase()
            database.botPresets.unshift(structuredClone(added))
            database.botPresetsId = 1
        })

        await activatePresetOverride(1)

        expect(runtime.readPreset).toHaveBeenCalledExactlyOnceWith('preset-b')
        expect(getEffectivePresetId()).toBe('preset-b')
        expect(getDatabase().mainPrompt).toBe('Prompt B')
    })

    it('keeps a loaded preset record that is edited while its stored value is read', async () => {
        stored.get('preset-b')!.mainPrompt = 'Stored B'
        const read = deferred()
        const readStored = runtime.readPreset.getMockImplementation()!
        runtime.readPreset.mockImplementation(async (id) => {
            await read.promise
            return readStored(id)
        })

        const activation = activatePresetOverride(1)
        await vi.waitFor(() => expect(runtime.readPreset).toHaveBeenCalledOnce())
        const record = getDatabase().botPresets[1]
        expect(isLoadedPreset(record)).toBe(true)
        record.mainPrompt = 'Edited B'
        read.resolve()
        await activation

        expect(getDatabase().botPresets[1]).toBe(record)
        expect(record.mainPrompt).toBe('Edited B')
        expect(getEffectivePresetId()).toBe('preset-b')
        expect(getDatabase().mainPrompt).toBe('Edited B')
    })

    it('installs the stored value when the record is still a catalog stub', async () => {
        getDatabase().botPresets[1] = { id: 'preset-b', name: 'Preset B' } as botPreset
        expect(isLoadedPreset(getDatabase().botPresets[1])).toBe(false)

        await activatePresetOverride(1)

        expect(getDatabase().botPresets[1]).toEqual(stored.get('preset-b'))
        expect(getEffectivePresetId()).toBe('preset-b')
        expect(getDatabase().mainPrompt).toBe('Prompt B')
    })

    it('clears the override for a null index', async () => {
        setEffectivePresetOverride('preset-b')
        expect(getDatabase().mainPrompt).toBe('Prompt B')

        await activatePresetOverride(null)

        expect(runtime.flushPendingData).toHaveBeenCalledOnce()
        expect(runtime.readPreset).not.toHaveBeenCalled()
        expect(getEffectivePresetId()).toBe('preset-a')
        expect(getDatabase().mainPrompt).toBe('Prompt A')
    })

    it('rejects a missing preset without changing the override', async () => {
        await expect(activatePresetOverride(5)).rejects.toThrow('Preset was not found')
        expect(runtime.readPreset).not.toHaveBeenCalled()

        const presetB = stored.get('preset-b')!
        stored.delete('preset-b')
        await expect(activatePresetOverride(1)).rejects.toThrow('Preset was not found')

        stored.set('preset-b', presetB)
        runtime.flushPendingData.mockImplementation(async () => {
            getDatabase().botPresets.splice(1, 1)
        })
        await expect(activatePresetOverride(1)).rejects.toThrow('Preset was not found')

        expect(getEffectivePresetId()).toBe('preset-a')
        expect(getDatabase().mainPrompt).toBe('Prompt A')
    })
})


function selectBoundChat() {
    const db = getDatabase()
    db.characters = [{ chaId: 'owner', chatPage: 0, chats: [{ id: 'chat-a', bindedBotPreset: 'preset-b', message: [], note: '', name: 'A', localLore: [] }] }] as Database['characters']
    selectedCharID.set(0)
    return db.characters[0].chats[0]
}

it('hydrates a bound catalog entry before applying its model and prompt', async () => {
    const chat = selectBoundChat()
    stored.get('preset-b')!.aiModel = 'bound-model'
    getDatabase().botPresets[1] = { id: 'preset-b', name: 'Preset B' } as botPreset
    expect(await activateChatPresetBinding()).toBe(true)
    expect(getDatabase().mainPrompt).toBe('Prompt B')
    expect(getDatabase().aiModel).toBe('bound-model')
    expect(getDatabase().botPresetsId).toBe(0)
    chat.bindedBotPreset = ''
    expect(await activateChatPresetBinding()).toBe(true)
    expect(getDatabase().mainPrompt).toBe('Prompt A')
})

it('does not activate a delayed preset read after a different chat is selected', async () => {
    selectBoundChat()
    const db = getDatabase()
    db.botPresets[1] = { id: 'preset-b', name: 'Preset B' } as botPreset
    const gate = deferred()
    runtime.readPreset.mockImplementation(async () => { await gate.promise; return { revision: 1, value: stored.get('preset-b')! } })
    const activation = activateChatPresetBinding()
    await vi.waitFor(() => expect(runtime.readPreset).toHaveBeenCalled())
    db.characters[0].chats.push({ id: 'chat-b', message: [], note: '', name: 'B', localLore: [] })
    db.characters[0].chatPage = 1
    gate.resolve()
    expect(await activation).toBe(false)
    expect(db.mainPrompt).toBe('Prompt A')
})

it('retains a missing binding UUID while falling back to the global prompt', async () => {
    selectBoundChat().bindedBotPreset = 'missing'
    expect(await activateChatPresetBinding()).toBe(true)
    expect(getDatabase().mainPrompt).toBe('Prompt A')
    expect(getDatabase().characters[0].chats[0].bindedBotPreset).toBe('missing')
})


it('carries the bound preset through a working-set replacement and applies remote preset changes', async () => {
    selectBoundChat()
    await activateChatPresetBinding()
    const replacement = getDatabase({ snapshot: true })
    replacement.botPresets[1].mainPrompt = 'Received prompt'
    replacement.botPresets[1].aiModel = 'received-model'
    createProductionStateAdapter().replaceDatabase(replacement)
    expect(getEffectivePresetId()).toBe('preset-b')
    expect(getDatabase().botPresetsId).toBe(0)
    expect([getDatabase().mainPrompt, getDatabase().aiModel]).toEqual(['Received prompt', 'received-model'])
})


it('allows the request and lifecycle to load the same binding concurrently', async () => {
    selectBoundChat()
    getDatabase().botPresets[1] = { id: 'preset-b', name: 'Preset B' } as botPreset
    const gate = deferred()
    runtime.readPreset.mockImplementation(async () => { await gate.promise; return { revision: 1, value: stored.get('preset-b')! } })
    const first = activateChatPresetBinding()
    await vi.waitFor(() => expect(runtime.readPreset).toHaveBeenCalledTimes(1))
    const second = activateChatPresetBinding()
    await vi.waitFor(() => expect(runtime.readPreset).toHaveBeenCalledTimes(2))
    gate.resolve()
    expect(await Promise.all([first, second])).toEqual([true, true])
    expect(getDatabase().mainPrompt).toBe('Prompt B')
})
