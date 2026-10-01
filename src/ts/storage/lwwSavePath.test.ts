import { describe, expect, it, vi } from 'vitest'
import { IDBFactory, IDBKeyRange } from 'fake-indexeddb'
import { IndexedDbPersistentDataStore } from './indexedDbPersistentDataStore'
import { fixtureDatabase } from './tests/persistentDataFixtures'
import { SaveCoordinator } from './saveCoordinator'
import { capturePersistentRoot, createPersistentDataRuntime } from './persistentDataRuntime'
import { createGeneratingConversationRegistry } from './generatingConversationRegistry'
import { diffMaterializedCharacter, diffRecordCollection } from './persistentUnitCapture'
import { createConversationSummaryStubFromChat } from './conversationResidency'
import type { Database, character } from './database.svelte'
import type { LwwStageReceive, PersistentDataStore, WorkingSetCommit } from './persistentDataStore'

async function harness() {
    const database = structuredClone(fixtureDatabase)
    database.modules = []
    database.personas = []
    database.botPresetsId = 0
    database.selectedPersona = 0
    const store = new IndexedDbPersistentDataStore(crypto.randomUUID(), new IDBFactory(), IDBKeyRange)
    await store.open()
    const imported = await store.replaceFromDatabase(database)
    const commit = vi.spyOn(store, 'commit')
    const coordinator = new SaveCoordinator({ store,
        captureRoot: () => capturePersistentRoot(database), capturePresets: () => database.botPresets,
        captureCharacters: () => database.characters, captureSelectedCharacter: () => database.characters[0],
        captureCharacter: (id) => database.characters.find((value) => value.chaId === id) ?? null,
        replaceDatabase: () => undefined })
    coordinator.initialize(imported.revision, { ...database, ...capturePersistentRoot(database) } as unknown as Database)
    return { database, store, commit, coordinator }
}

async function runtimeHarness() {
    const values = await harness()
    const {database,store} = values
    const runtime = createPersistentDataRuntime({store,state:{
        captureRoot:()=>capturePersistentRoot(database), capturePresets:()=>database.botPresets,
        captureCharacters:()=>database.characters, captureWorkingSetDatabase:()=>database,
        captureSelectedCharacter:()=>database.characters[0], captureCharacter:(id)=>database.characters.find((value)=>value.chaId===id)??null,
        getSelectedCharacterId:()=>database.characters[0]?.chaId,replaceDatabase:()=>undefined,publishCharacter:()=>undefined,publishConversation:()=>undefined,
    },prepareDatabase:async(value)=>value})
    await runtime.initializeActiveWorkingSet(database)
    return {...values,store:store as PersistentDataStore,runtime}
}

describe('LWW renderer save path', () => {
    it('adopts accepted remote units without re-emitting them or resetting unrelated local edits', async () => {
        const {database,store,runtime,commit} = await runtimeHarness()
        store.lwwStageReceive = async()=>undefined
        store.lwwApplyReceive = async()=>{
            const revision=(await store.readRoot()).revision
            const result=await store.commit({expectedRevision:revision,unitMutations:[{key:'["character","char-a","notes"]',type:'set',value:'remote'}]})
            return {...result,affectedKeys:['["character","char-a","notes"]'],heldKeys:[],deferredKeys:[]}
        }
        store.lwwFinishReceive=async()=>undefined
        await runtime.applyLwwReceive({bindingAuthority:'a',requestId:'r',changes:[],progress:{kind:'server',cursor:'1'},admittedTimeUpperMs:'100'})
        expect((database.characters[1] as character).notes).toBe('remote')
        commit.mockClear()
        await runtime.flushPendingDataLocally('after-remote')
        expect(commit).not.toHaveBeenCalled()
    })

    it('finishes durable receive progress when renderer projection fails and requires refresh', async () => {
        const {store,runtime}=await runtimeHarness()
        store.lwwStageReceive=async()=>undefined
        store.lwwApplyReceive=async()=>({revision:runtime.revision,affectedKeys:['["character","char-a","notes"]'],heldKeys:[],deferredKeys:[]})
        store.lwwFinishReceive=vi.fn(async()=>undefined)
        vi.spyOn(store,'acquireRevision').mockRejectedValueOnce(new Error('projection unavailable'))
        await expect(runtime.applyLwwReceive({bindingAuthority:'a',requestId:'r',changes:[],progress:{kind:'server',cursor:'1'},admittedTimeUpperMs:'100'})).rejects.toThrow('projection unavailable')
        expect(store.lwwFinishReceive).toHaveBeenCalledOnce()
        expect(runtime.pendingWorkingSetRefreshRevision).toBe(runtime.revision)
    })

    it('commits plugin units once if their pinned projection loses its revision', async () => {
        const {store,runtime,commit}=await runtimeHarness()
        vi.spyOn(store,'acquireRevision').mockRejectedValueOnce(new Error('pin lost'))
        await expect(runtime.commitPersistentUnitIntent('plugin',[{key:'["character","char-a","notes"]',type:'set',value:'durable'}])).rejects.toThrow('pin lost')
        expect(commit).toHaveBeenCalledOnce()
        expect((await store.readCharacter('char-a'))?.value['notes']).toBe('durable')
        expect(runtime.pendingWorkingSetRefreshRevision).toBe(runtime.revision)
    })

    it('captures fields and conversations of two characters independently of selection', async () => {
        const { database, store, commit, coordinator } = await harness()
        ;(database.characters[0] as character).notes = 'first field'
        ;(database.characters[1] as character).notes = 'second field'
        database.characters[0].chats[0].note = 'first metadata'
        database.characters[1].chats[0].message.push({ role: 'char', data: 'second reply', chatId: 'kept-message-id' })
        coordinator.markPersistentDataDirty(0)
        await coordinator.flushPendingDataLocally('two-characters')
        expect(((await store.readCharacter('char-b'))?.value as character).notes).toBe('first field')
        expect(((await store.readCharacter('char-a'))?.value as character).notes).toBe('second field')
        expect((await store.readConversation('char-b', 'conv-beta'))?.value.note).toBe('first metadata')
        expect((await store.readConversation('char-a', 'conv-long'))?.value.message.at(-1)?.chatId).toBe('kept-message-id')
        expect(commit.mock.calls[0][0]).not.toHaveProperty('replaceCharacter')
        expect(commit.mock.calls[0][0].unitMutations?.map((value) => value.key)).toEqual(expect.arrayContaining([
            '["character","char-b","notes"]', '["character","char-a","notes"]', '["conversation","char-b","conv-beta","note"]',
        ]))
        commit.mockClear()
        coordinator.markPersistentDataDirty(0)
        await coordinator.flushPendingDataLocally('unchanged')
        expect(commit).not.toHaveBeenCalled()
    })

    it('retries changed units onto a newer revision without replacing unrelated values', async () => {
        const { database, store, coordinator } = await harness()
        await store.commit({ expectedRevision: coordinator.revision, unitMutations: [
            { key: '["character","char-a","desc"]', type: 'set', value: 'intervening description' },
        ] })
        ;(database.characters[1] as character).notes = 'intentional note'
        await coordinator.flushPendingDataLocally('revision-race')
        const current = (await store.readCharacter('char-a'))!.value as character
        expect(current.notes).toBe('intentional note')
        expect(current.desc).toBe('intervening description')
    })

    it('does not replace unloaded conversation bodies with empty catalog placeholders', async () => {
        const { database, store, coordinator } = await harness()
        const original = database.characters[1].chats[0]
        database.characters[1].chats[0] = createConversationSummaryStubFromChat('char-a', original, 0)
        coordinator.initialize(coordinator.revision, database)
        ;(database.characters[1] as character).notes = 'detail only'
        await coordinator.flushPendingDataLocally('catalog-shell')
        expect((await store.readConversation('char-a', 'conv-long'))?.value.message).toHaveLength(130)
    })

    it('emits only one changed generic record and permits plugin reinstall', async () => {
        expect(diffRecordCollection('plugins', [{ name: 'a', source: 'old' }, { name: 'b', source: 'untouched' }],
            [{ name: 'a', source: 'new' }, { name: 'b', source: 'untouched' }])).toEqual([
            { key: '["record","plugins","a"]', type: 'set', value: { name: 'a', source: 'new' } },
        ])
        const { store, coordinator } = await harness()
        await coordinator.commitPersistentUnitIntent('install', [{ key: '["record","plugins","owner"]', type: 'set', value: { name: 'owner', source: 'first' } }])
        await coordinator.commitPersistentUnitIntent('remove', [{ key: '["record","plugins","owner"]', type: 'delete' }])
        await coordinator.commitPersistentUnitIntent('reinstall', [{ key: '["record","plugins","owner"]', type: 'set', value: { name: 'owner', source: 'second' } }])
        expect((await store.readRoot()).value.plugins).toEqual([{ name: 'owner', source: 'second' }])
    })

    it('keeps group membership atomic and excludes unchanged fields', () => {
        const before = { ...fixtureDatabase.characters[0], type: 'group', characters: ['a'], characterTalks: [1], characterActive: [true] } as unknown as Database['characters'][number]
        const after = structuredClone(before)
        Object.assign(after, { characters: ['b'], characterTalks: [2], characterActive: [false] })
        expect(diffMaterializedCharacter(before, after).unitMutations).toEqual([
            { key: '["group-members","char-b"]', type: 'set', value: { characters: ['b'], characterTalks: [2], characterActive: [false] } },
        ])
    })

    it('pins export under paused writes while later saves wait', async () => {
        const { database, coordinator, commit } = await harness()
        let release!: () => void
        let entered!: () => void
        const waiting = new Promise<void>((resolve) => { release = resolve })
        const started = new Promise<void>((resolve) => { entered = resolve })
        const exportJob = coordinator.withPausedPersistentWrites('export', async (token) => { entered(); await waiting; return token.revision })
        await started
        database.username = 'later'
        const save = coordinator.flushPendingDataLocally('later-save')
        expect(commit).not.toHaveBeenCalled()
        release()
        await exportJob
        await save
        expect(commit).toHaveBeenCalledOnce()
    })

    it('registers all generating conversations using both stable IDs', () => {
        const registry = createGeneratingConversationRegistry()
        const releaseA = registry.register({ characterId: 'a', conversationId: 'same' })
        const releaseB = registry.register({ characterId: 'b', conversationId: 'same' })
        const secondA = registry.register({ characterId: 'a', conversationId: 'same' })
        releaseA()
        expect(registry.snapshot()).toHaveLength(2)
        secondA()
        expect(registry.snapshot()).toEqual([{ characterId: 'b', conversationId: 'same' }])
        releaseB()
        expect(registry.snapshot()).toEqual([])
    })

    it('flushes capture before native apply and refreshes unselected fields without initializing again', async () => {
        const { database, store, coordinator } = await harness()
        const registry = createGeneratingConversationRegistry()
        registry.register({ characterId: 'char-a', conversationId: 'conv-long' })
        const stage = vi.fn(async () => {
            expect(((await store.readCharacter('char-b'))?.value as character).notes).toBe('uncaptured local')
        })
        const apply = vi.fn(async () => {
            const revision = (await store.readRoot()).revision
            const result = await store.commit({ expectedRevision: revision, unitMutations: [{ key: '["character","char-a","desc"]', type: 'set', value: 'remote field' }] })
            return { revision: result.revision, affectedKeys: ['["character","char-a","desc"]'], heldKeys: [], deferredKeys: ['["messages","char-a","conv-long"]'] }
        })
        const finish = vi.fn(async () => undefined)
        const native = store as PersistentDataStore
        native.lwwStageReceive = stage
        native.lwwApplyReceive = apply
        native.lwwFinishReceive = finish
        const runtime = createPersistentDataRuntime({ store: native, state: {
            captureRoot: () => capturePersistentRoot(database), capturePresets: () => database.botPresets,
            captureCharacters: () => database.characters, captureSelectedCharacter: () => database.characters[0],
            captureCharacter: (id) => database.characters.find((value) => value.chaId === id) ?? null,
            captureWorkingSetDatabase: () => database, getSelectedCharacterId: () => 'char-b',
            getGeneratingConversations: () => registry.snapshot(), replaceDatabase: () => undefined,
            publishCharacter: () => undefined, publishConversation: () => undefined,
        }, prepareDatabase: async (value) => value })
        await runtime.initializeActiveWorkingSet(database)
        const initialize = vi.spyOn(SaveCoordinator.prototype, 'initialize')
        ;(database.characters[0] as character).notes = 'uncaptured local'
        await runtime.applyLwwReceive({ bindingAuthority: '1', requestId: 'receive', changes: [], progress: { kind: 'server', cursor: '1' }, admittedTimeUpperMs: '100' } as LwwStageReceive)
        expect(apply).toHaveBeenCalledWith({ bindingAuthority: '1', requestId: 'receive', generating: [{ characterId: 'char-a', conversationId: 'conv-long' }] })
        expect((database.characters[1] as character).desc).toBe('remote field')
        expect((database.characters[0] as character).notes).toBe('uncaptured local')
        expect(finish).toHaveBeenCalledOnce()
        expect(initialize).not.toHaveBeenCalled()
        initialize.mockRestore()
    })
})
