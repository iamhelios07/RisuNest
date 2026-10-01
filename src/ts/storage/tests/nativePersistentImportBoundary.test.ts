import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'

import type { Database } from '../database.svelte'
import type { PersistentRoot } from '../persistentDataStore'
import { canonicalJson } from '../saveCoordinator'

type StagedDatabase = {
    root: PersistentRoot | null
    presets: Database['botPresets']
    characters: Database['characters']
}

class InMemoryNativeCommandDouble {
    revision = 0
    database = { characters: [] } as unknown as Database
    rejectNextCharacterBatch = false
    private staging = new Map<string, StagedDatabase>()
    private stagingSequence = 0

    get stagedDatabaseCount(): number {
        return this.staging.size
    }

    async invoke(command: string, args: Record<string, unknown> = {}): Promise<unknown> {
        switch (command) {
            case 'pds_open':
                return { revision: this.revision }
            case 'pds_read_root': {
                const { characters: _characters, botPresets: _botPresets, ...root } = this.database
                return { revision: this.revision, value: structuredClone(root) }
            }
            case 'pds_replace_begin': {
                const stagingId = `staging-${++this.stagingSequence}`
                this.staging.set(stagingId, { root: null, presets: [], characters: [] })
                return { stagingId }
            }
            case 'pds_replace_put_root': {
                const staging = this.requireStaging(args.stagingId)
                staging.root = structuredClone(args.root as PersistentRoot)
                return undefined
            }
            case 'pds_replace_put_presets': {
                const staging = this.requireStaging(args.stagingId)
                staging.presets = structuredClone(args.presets as Database['botPresets'])
                return undefined
            }
            case 'pds_replace_add_characters': {
                if (this.rejectNextCharacterBatch) {
                    this.rejectNextCharacterBatch = false
                    throw new Error('character batch rejected')
                }
                const staging = this.requireStaging(args.stagingId)
                staging.characters.push(...structuredClone(args.characters as Database['characters']))
                return undefined
            }
            case 'pds_replace_preserve_repositories': {
                const expectedRevision = args.expectedRevision as number | undefined
                if (expectedRevision !== undefined && expectedRevision !== this.revision) {
                    throw { code: 'revision-conflict', expected: expectedRevision, actual: this.revision }
                }
                return { revision: this.revision }
            }
            case 'pds_replace_commit': {
                const stagingId = args.stagingId as string
                const staging = this.requireStaging(stagingId)
                const expectedRevision = args.expectedRevision as number | undefined
                if (expectedRevision !== undefined && expectedRevision !== this.revision) {
                    throw { code: 'revision-conflict', expected: expectedRevision, actual: this.revision }
                }
                if (!staging.root) throw new Error('staged root is required')
                this.database = {
                    ...structuredClone(staging.root),
                    botPresets: structuredClone(staging.presets),
                    characters: structuredClone(staging.characters),
                } as Database
                this.staging.delete(stagingId)
                this.revision++
                return { revision: this.revision }
            }
            case 'pds_replace_abort':
                this.staging.delete(args.stagingId as string)
                return undefined
            case 'pds_materialize':
                return structuredClone(this.database)
            default:
                throw new Error(`Unexpected native command: ${command}`)
        }
    }

    private requireStaging(value: unknown): StagedDatabase {
        const staging = this.staging.get(value as string)
        if (!staging) throw new Error('staging database not found')
        return staging
    }
}

const native = vi.hoisted(() => ({ boundary: null as unknown as InMemoryNativeCommandDouble }))
const platform = vi.hoisted(() => ({ isTauri: true }))

vi.mock('../../platform', () => platform)
vi.mock('@tauri-apps/api/core', () => ({
    invoke: (command: string, args?: Record<string, unknown>) => native.boundary.invoke(command, args),
}))

import { installLocalBackup } from '../databaseRestore'
import { bootstrapPersistentDatabase } from '../persistentBootstrap'
import { createPersistentDataRuntime, type PersistentDataRuntimeStateAdapter } from '../persistentDataRuntime'
import { createPersistentDataStore } from '../persistentDataStoreFactory'
import { SqlitePersistentDataStore } from '../sqlitePersistentDataStore'
import { fixtureDatabase } from './persistentDataFixtures'

function createStateAdapter(initial: Database): PersistentDataRuntimeStateAdapter & { current(): Database } {
    let database = structuredClone(initial)
    return {
        current: () => database,
        captureRoot: () => {
            const { characters: _characters, botPresets: _botPresets, ...root } = database
            return structuredClone(root)
        },
        capturePresets: () => structuredClone(database.botPresets ?? []),
        captureSelectedCharacter: () => structuredClone(database.characters[0] ?? null),
        captureCharacter: (id) => {
            const character = database.characters.find((item) => item.chaId === id)
            return character ? structuredClone(character) : null
        },
        getSelectedCharacterId: () => database.characters[0]?.chaId,
        replaceDatabase: (replacement) => {
            database = structuredClone(replacement)
        },
        publishCharacter: () => undefined,
        publishConversation: () => undefined,
    }
}

describe('simulated native persistent local backup boundary', () => {
    let indexedDbOpen: ReturnType<typeof vi.fn>

    beforeEach(() => {
        native.boundary = new InMemoryNativeCommandDouble()
        platform.isTauri = true
        indexedDbOpen = vi.fn(() => {
            throw new Error('IndexedDB must not open in Tauri')
        })
        vi.stubGlobal('indexedDB', { open: indexedDbOpen })
    })

    afterEach(() => {
        vi.unstubAllGlobals()
        vi.clearAllMocks()
    })

    test('converts upstream character formats at explicit import without migrating an active store', async () => {
        const upstream = structuredClone(fixtureDatabase)
        delete (upstream as Partial<Database>).formatversion
        upstream.characters[0].image = 'C:\\synthetic\\assets\\avatar.png'
        upstream.characters[0].emotionImages = [['happy','C:\\synthetic\\assets\\happy.png']]
        native.boundary.database = structuredClone(upstream)
        native.boundary.revision = 1
        const prepareDatabase = vi.fn(async(input:Database)=>{
            const database = structuredClone(input)
            database.formatversion = 5
            database.characters[0].image = 'assets/avatar.png'
            database.characters[0].emotionImages = [['happy','assets/happy.png']]
            return database
        })
        const store = createPersistentDataStore()
        const replacement = vi.spyOn(store,'replaceFromDatabase')
        const boot = await bootstrapPersistentDatabase({store,prepareDatabase:async(input)=>({database:await prepareDatabase(input),changed:true})})
        expect(boot.revision).toBe(1)
        expect(replacement).not.toHaveBeenCalled()
        expect((await store.materializeDatabase()).characters[0].image).toBe(upstream.characters[0].image)
        const runtime = createPersistentDataRuntime({store,state:createStateAdapter(boot.database),prepareDatabase})
        await runtime.initializeActiveWorkingSet(boot.database)
        await installLocalBackup(upstream,{replaceDatabase:runtime.replacePersistentDatabase,publishAcceptedRevision:async()=>undefined,relaunch:async()=>undefined})
        const imported = await store.materializeDatabase()
        expect(imported.formatversion).toBe(5)
        expect(imported.characters[0]).toMatchObject({image:'assets/avatar.png',emotionImages:[['happy','assets/happy.png']]})
        expect(replacement).toHaveBeenCalledOnce()
        expect(native.boundary.revision).toBe(2)
    })

    test('bootstraps a fresh Tauri store, imports a local backup, and preserves it on failure', async () => {
        const preparedDefault = { characters: [], botPresets: [] } as unknown as Database
        const prepareDatabase = async (database: Database) => database.characters
            ? structuredClone(database)
            : structuredClone(preparedDefault)
        const store = createPersistentDataStore()
        const prepareBootstrap = async (input: Database) => {
            const database = await prepareDatabase(input)
            return {
                database,
                changed: canonicalJson(database) !== canonicalJson(input),
            }
        }
        const result = await bootstrapPersistentDatabase({
            store,
            prepareDatabase: prepareBootstrap,
        })

        expect(store).toBeInstanceOf(SqlitePersistentDataStore)
        expect(result.revision).toBe(1)
        expect(result.database).toEqual(preparedDefault)
        expect(indexedDbOpen).not.toHaveBeenCalled()
        expect(await store.materializeDatabase()).toEqual(result.database)

        const state = createStateAdapter(result.database)
        const runtime = createPersistentDataRuntime({ store, state, prepareDatabase })
        await runtime.initializeActiveWorkingSet(result.database)
        const events: string[] = []
        const publishAcceptedRevision = async () => {
            expect(state.current()).toEqual(fixtureDatabase)
            expect(await store.materializeDatabase()).toEqual(fixtureDatabase)
            events.push('publish')
        }
        const relaunch = async () => {
            expect(events).toEqual(['publish'])
            events.push('relaunch')
        }

        await installLocalBackup(structuredClone(fixtureDatabase), {
            replaceDatabase: runtime.replacePersistentDatabase,
            publishAcceptedRevision,
            relaunch,
        })

        expect(state.current()).toEqual(fixtureDatabase)
        expect(await store.materializeDatabase()).toEqual(fixtureDatabase)
        expect(events).toEqual(['publish', 'relaunch'])

        const reopened = new SqlitePersistentDataStore()
        const reopenedResult = await bootstrapPersistentDatabase({
            store: reopened,
            prepareDatabase: prepareBootstrap,
        })
        expect(reopenedResult.database).toEqual(fixtureDatabase)

        const rejectedCandidate = structuredClone(fixtureDatabase)
        rejectedCandidate.username = 'Rejected local backup'
        native.boundary.rejectNextCharacterBatch = true

        await expect(installLocalBackup(rejectedCandidate, {
            replaceDatabase: runtime.replacePersistentDatabase,
            publishAcceptedRevision,
            relaunch,
        })).rejects.toThrow('character batch rejected')

        expect(state.current()).toEqual(fixtureDatabase)
        expect(await store.materializeDatabase()).toEqual(fixtureDatabase)
        expect(events).toEqual(['publish', 'relaunch'])
        expect(native.boundary.stagedDatabaseCount).toBe(0)
    })
})
