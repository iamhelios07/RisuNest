import { RevisionConflictError, SnapshotReleasedError, type PersistentDataStore, type AssetAlias } from '../../src/ts/storage/persistentDataStore'
import type { Database } from '../../src/ts/storage/database.svelte'
import { normalizeConversationContextInput, readPinnedConversationContext } from '../../src/ts/plugins/conversationContext'

const canonical = (value: unknown): string => JSON.stringify(value, (_key, item) =>
    item && typeof item === 'object' && !Array.isArray(item)
        ? Object.fromEntries(Object.keys(item).sort().map(key => [key, item[key]])) : item)
function equal(actual: unknown, expected: unknown, label: string) {
    if (canonical(actual) !== canonical(expected)) throw new Error(label)
}
const largeText = 'synthetic'.repeat(160_000)
const aliases: AssetAlias[] = ['a', 'b'].map(key => ({ kind: 'asset', key: `assets/${key}.bin`, objectHash: null, size: 0, mime: '', name: key, ext: 'bin', metadata: {} }))

export async function verifyContractReadback(store: PersistentDataStore): Promise<void> {
    const root = await store.readRoot()
    equal(root.value.username, 'contract-committed', 'delta survives reopen')
    equal((root.value as typeof root.value & { syntheticLarge?: string }).syntheticLarge, largeText, 'large transport survives reopen')
    equal((await store.readAssetAliasesByKeys('asset', aliases.map(alias => alias.key))).value, aliases, 'aliases survive reopen')
}

async function readContext(store: PersistentDataStore, characterId: string, conversationId: string, revision: number, beforeCharacter?: () => Promise<void>) {
    const lease = await store.acquireRevision(revision)
    const reader = new Proxy(lease, { get(target, key) {
        const value = Reflect.get(target, key, target)
        if (typeof value !== 'function') return value
        if (key !== 'readCharacter' || !beforeCharacter) return value.bind(target)
        return async (...args: unknown[]) => {
            const hook = beforeCharacter
            beforeCharacter = undefined
            await hook!()
            return value.apply(target, args)
        }
    } })
    try {
        return await readPinnedConversationContext(reader, normalizeConversationContextInput({
            characterId, conversationId, include: { character: true, lore: true }, messages: { limit: 128 },
        }), { selected: null, allowPrivate: true })
    } finally { await lease.release() }
}

export async function verifyPinnedConversationContext(store: PersistentDataStore, fixture: Database): Promise<void> {
    const character = fixture.characters[1]
    const chat = character.chats[1]
    const head = (await store.readRoot()).revision
    let committed = false
    const pinned = await readContext(store, character.chaId, chat.id!, head, async () => {
        const metadata = await store.readConversationMetadata(character.chaId, chat.id!)
        await store.commit({ expectedRevision: head,
            rootMutations: [{ type: 'set', key: 'syntheticContext', value: 'late' }],
            conversations: [{ type: 'replace-range', characterId: character.chaId, conversationId: chat.id!,
                start: chat.message.length, deleteCount: 0, messages: [{ role: 'char', data: 'late', chatId: 'context-late' }],
                conversation: { ...metadata!.value.conversation, name: 'late name' } }] })
        committed = true
    })
    if (!committed) throw new Error('context commit did not run')
    equal([pinned?.revision, pinned?.characterIndex, pinned?.chatIndex, pinned?.messageCount], [head, 1, 1, chat.message.length], 'context pinned position')
    equal([pinned?.character?.chaId, pinned?.conversation.name], [character.chaId, chat.name], 'context pinned records')
    equal(pinned?.messages?.messages.map(message => message.chatId), chat.message.map(message => message.chatId), 'context pinned window')
    const after = await readContext(store, character.chaId, chat.id!, head + 1)
    equal([after?.revision, after?.messageCount, after?.conversation.name], [head + 1, chat.message.length + 1, 'late name'], 'context after commit')
}

export async function runContractScenarios(store: PersistentDataStore, fixture: Database, reopen: () => Promise<PersistentDataStore>): Promise<void> {
    const imported = await store.replaceFromDatabase(structuredClone(fixture))
    const before = await store.readRoot()
    await store.commitWorkingSetChangeCursor?.(imported.revision)
    const lease = await store.acquireRevision(imported.revision)
    try {
        const committed = await store.commit({ expectedRevision: imported.revision,
            rootMutations: [{ type: 'set', key: 'username', value: 'contract-committed' }], assetAliases: aliases })
        equal(committed.revision, imported.revision + 1, 'delta revision advances once')
        equal(await lease.readRoot(), before, 'lease isolates root from later commit')
        equal((await lease.listAssetAliases({ limit: 1 })).items, [], 'lease isolates aliases')
        let rejected: unknown
        try { await store.commit({ expectedRevision: imported.revision, rootMutations: [{ type: 'set', key: 'username', value: 'rejected' }] }) }
        catch (error) { rejected = error }
        if (!(rejected instanceof RevisionConflictError)) throw new Error('delta conflict type')
        equal([rejected.expectedRevision, rejected.actualRevision], [imported.revision, committed.revision], 'delta conflict fields')
        const first = await store.queryCharacters({ order: 'configured', trash: false, limit: 1 })
        if (!first.nextCursor) throw new Error('character cursor missing')
        const second = await store.queryCharacters({ order: 'configured', trash: false, limit: 1, cursor: first.nextCursor })
        equal([...first.items, ...second.items].map(item => item.id), fixture.characters.slice(0, 2).map(item => item.chaId), 'character pagination')
        const aliasPage = await store.listAssetAliases({ limit: 1 })
        if (!aliasPage.nextCursor) throw new Error('asset cursor missing')
        const aliasTail = await store.listAssetAliases({ limit: 1, cursor: aliasPage.nextCursor })
        equal([...aliasPage.items, ...aliasTail.items], aliases, 'asset pagination')
        const current = await store.acquireRevision(committed.revision)
        try {
            if (current.readWorkingSetChangePage) {
                const changes = await current.readWorkingSetChangePage(imported.revision, null, 1)
                equal(changes.length, 1, 'change page bound')
                const tail = await current.readWorkingSetChangePage(imported.revision, changes[0], 1)
                if (tail.some(item => canonical(item) === canonical(changes[0]))) throw new Error('change cursor repeated')
            }
        } finally { await current.release() }
        await store.commit({ expectedRevision: committed.revision, rootMutations: [{ type: 'set', key: 'syntheticLarge', value: largeText }] })
    } finally { await lease.release() }
    let released: unknown
    try { await lease.readRoot() } catch (error) { released = error }
    if (!(released instanceof SnapshotReleasedError)) throw new Error('released lease still readable')
    await verifyPinnedConversationContext(store, fixture)
    await verifyContractReadback(await reopen())
}
