import {beforeEach,describe,expect,it,vi} from 'vitest'
vi.mock('../platform',()=>({isTauri:true}))
const binding=vi.hoisted(()=>({bound:true,state:{targetAuthority:'9007199254740993'},fence:vi.fn(async()=>{}),assertAuthority:vi.fn(async()=>{}),resume:vi.fn(async()=>{})}))
vi.mock('./sync/bindingRegistry',()=>({prepareBoundLibraryReplacement:async()=>binding}))
vi.mock('../alert',()=>({alertCheckboxConfirm:async()=>({confirmed:true,checked:true})}))
vi.mock('../plugins/apiV3/v3.svelte',()=>({fencePluginExecutionForAuthorityReplacement:async()=>{},invalidatePluginCachesAfterAuthorityReplacement:async()=>{},restartPluginsAfterAuthorityReplacement:async()=>{}}))
import type {Database} from './database.svelte'
import type {PersistentDataStore,PersistentRevisionLease} from './persistentDataStore'
import {capturePersistentPluginStorage,capturePersistentPresets,capturePersistentRoot,createPersistentDataRuntime} from './persistentDataRuntime'
import type {OfficialRevisionPublisher} from './saveCoordinator'
import {makeDatabase} from './saveCoordinator.testSupport'
import {retryCommittedWorkingSetRefreshWithContinuation} from './committedWorkingSetContinuation'
async function createHarness(officialPublisher?: OfficialRevisionPublisher, captureWorkingSet = false) {
    let database = makeDatabase()
    let durable = structuredClone(database)
    let revision = 1
    let generating = false
    const leases: PersistentRevisionLease[] = []
    const replaceDatabase = vi.fn((replacement: Database) => { database = replacement })
    const onWorkingSetRefreshRequired = vi.fn()
    const onBackgroundError = vi.fn()
    const publishPresetWorkingSet = vi.fn()
    const store = {
        open: vi.fn(async () => undefined),
        readRoot: vi.fn(async () => ({ revision, value: capturePersistentRoot(durable) })),
        commit: vi.fn(async () => { throw new Error('Unexpected content commit') }),
        replaceFromDatabase: vi.fn(async (replacement: Database, expected: number) => {
            expect(expected).toBe(revision)
            durable = structuredClone(replacement)
            return { revision: ++revision }
        }),
        acquireRevision: vi.fn(async (expected: number) => {
            expect(expected).toBe(revision)
            const snapshot = structuredClone(durable)
            const lease = {
                revision: expected,
                readRoot: vi.fn(async () => ({ revision: expected, value: capturePersistentRoot(snapshot) })),
                queryPresets: vi.fn(async () => ({ revision: expected, items: [] })),
                readPreset: vi.fn(async () => null),
                queryCharacters: vi.fn(async ({ trash }: { trash: boolean }) => ({
                    revision: expected,
                    items: trash ? [] : snapshot.characters.map((character, configuredIndex) => ({
                        id: character.chaId,
                        name: character.name,
                        type: character.type,
                        configuredIndex,
                        recentAt: 0,
                        trashed: false,
                        conversationCount: 0,
                    })),
                })),
                readCharacterSummary: vi.fn(async (id: string) => {
                    const configuredIndex = snapshot.characters.findIndex((character) => character.chaId === id)
                    if (configuredIndex < 0) return null
                    const character = snapshot.characters[configuredIndex]
                    return {
                        id, name: character.name, type: character.type, configuredIndex,
                        recentAt: 0, trashed: false, conversationCount: 0,
                    }
                }),
                readCharacter: vi.fn(async (id: string) => {
                    const value = snapshot.characters.find((character) => character.chaId === id)
                    if (!value) return null
                    const { chats: _chats, ...detail } = value
                    return { revision: expected, value: detail }
                }),
                queryConversations: vi.fn(async () => ({ revision: expected, items: [] })),
                queryPluginStorage: vi.fn(async () => ({ revision: expected, items: [] })),
                release: vi.fn(async () => undefined),
            } as unknown as PersistentRevisionLease
            leases.push(lease)
            return lease
        }),
        materializeDatabase: vi.fn(async () => { throw new Error('Unexpected complete materialization') }),
    } as unknown as PersistentDataStore
    const runtime = createPersistentDataRuntime({
        store,
        state: {
            captureWorkingSetDatabase: captureWorkingSet ? () => database : undefined,
            captureRoot: () => capturePersistentRoot(database),
            capturePluginStorage: () => capturePersistentPluginStorage(database),
            capturePresets: () => capturePersistentPresets(database),
            captureSelectedCharacter: () => database.characters[0] ?? null,
            captureCharacter: (id) => database.characters.find((value) => value.chaId === id) ?? null,
            getSelectedCharacterId: () => database.characters[0]?.chaId,
            getSelectedConversationId: () => null,
            replaceDatabase,
            publishCharacter: vi.fn(),
            publishConversation: vi.fn(),
            publishPresetWorkingSet,
            isConversationOperationActive: () => generating,
        },
        prepareDatabase: async (value) => structuredClone(value),
        officialPublisher,
        onWorkingSetRefreshRequired,
        onBackgroundError,
        clock: { setTimeout: vi.fn(() => 1), clearTimeout: vi.fn() },
    })
    await runtime.initializeActiveWorkingSet(database)
    return {
        runtime, store, replaceDatabase, publishPresetWorkingSet,
        onWorkingSetRefreshRequired, onBackgroundError, leases,
        get database() { return database },
        get durable() { return durable },
        set generating(value: boolean) { generating = value },
        nativeCommit(replacement: Database) {
            durable = structuredClone(replacement)
            return ++revision
        },
    }
}

beforeEach(()=>{binding.bound=true;binding.fence.mockClear();binding.assertAuthority.mockClear();binding.resume.mockClear()})
describe('direct native upstream import publication',()=>{
    it.each([{expectedRevision:99},{expectedMutationGeneration:99}])('preserves snapshot expectation before direct native staging: %j',async expectation=>{
        const h=await createHarness()
        await expect(h.runtime.replacePersistentDatabase({...makeDatabase(),username:'Rejected'},'stale-snapshot',{upstreamImport:true,...expectation})).rejects.toThrow()
        expect(h.store.replaceFromDatabase).not.toHaveBeenCalled()
        expect(h.database.username).not.toBe('Rejected')
        expect(()=>h.runtime.markPersistentDataDirty(1)).not.toThrow()
        expect(binding.resume).toHaveBeenCalledOnce()
    })
    it.each(['refresh','uncertain'] as const)('retains read-only activation and resumes only after strict recovery: %s',async failure=>{
        const h=await createHarness({pin:async()=>({publish:async()=>{},dispose:async()=>{}})})
        const incoming={...makeDatabase(),username:'Imported after recovery'}
        if(failure==='refresh') h.replaceDatabase.mockImplementationOnce(()=>{throw new Error('refresh failed')})
        else vi.mocked(h.store.replaceFromDatabase).mockImplementationOnce(async database=>{
            h.nativeCommit(database)
            throw new Error('activation result unavailable')
        })
        await expect(h.runtime.replacePersistentDatabase(incoming,'upstream-import',{upstreamImport:true,publishOfficial:true})).rejects.toThrow()
        expect(()=>h.runtime.markPersistentDataDirty(1)).toThrow()
        expect(binding.resume).not.toHaveBeenCalled()
        await expect(retryCommittedWorkingSetRefreshWithContinuation(h.runtime)).resolves.toEqual({kind:'committed',revision:2,projection:'applied'})
        expect(h.database.username).toBe('Imported after recovery')
        expect(binding.resume).toHaveBeenCalledOnce()
        expect(h.runtime.hasPendingOfficialPublication()).toBe(true)
        await h.runtime.flushPendingDataLocally('recovery-baselines')
        expect(h.store.commit).not.toHaveBeenCalled()
    })
    it.each([true,false])('uses captured LWW authority and preserves explicit official publication=%s',async publishOfficial=>{
        const publication={publish:vi.fn(async()=>{}),dispose:vi.fn(async()=>{})}
        const pin=vi.fn(async()=>publication)
        const h=await createHarness({pin})
        const incoming={...makeDatabase(),username:'Upstream import'}
        await expect(h.runtime.replacePersistentDatabase(incoming,'upstream-import',{upstreamImport:true,publishOfficial})).resolves.toEqual({kind:'committed',revision:2,projection:'applied'})
        expect(h.store.replaceFromDatabase).toHaveBeenCalledWith(expect.objectContaining({username:'Upstream import',explicitGlobalChatVariables:{}}),1,[],undefined,
            {bindingAuthority:'9007199254740993',requestId:expect.any(String)})
        expect(binding.fence).toHaveBeenCalledOnce()
        expect(binding.resume).toHaveBeenCalledOnce()
        expect(h.database.username).toBe('Upstream import')
        expect(h.runtime.hasPendingOfficialPublication()).toBe(publishOfficial)
        if(publishOfficial){
            await h.runtime.publishCurrentOfficialRevision()
            expect(pin).toHaveBeenCalledExactlyOnceWith(2)
            expect(publication.publish).toHaveBeenCalledOnce()
        }else expect(pin).not.toHaveBeenCalled()
        await h.runtime.flushPendingDataLocally('import-baseline')
        expect(h.store.commit).not.toHaveBeenCalled()
    })
    it('clears stale pending official publication on a later upstream import without that option',async()=>{
        const pin=vi.fn(async()=>({publish:async()=>{},dispose:async()=>{}}))
        const h=await createHarness({pin})
        await h.runtime.replacePersistentDatabase({...makeDatabase(),username:'First'},'first',{upstreamImport:true,publishOfficial:true})
        expect(h.runtime.hasPendingOfficialPublication()).toBe(true)
        await h.runtime.replacePersistentDatabase({...makeDatabase(),username:'Second'},'second',{upstreamImport:true,publishOfficial:false})
        expect(h.runtime.hasPendingOfficialPublication()).toBe(false)
        expect(pin).not.toHaveBeenCalled()
        expect(h.database.username).toBe('Second')
    })
})
