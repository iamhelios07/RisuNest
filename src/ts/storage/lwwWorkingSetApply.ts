import { persistentRootFields } from './persistentRootFields'
import { createCatalogCharacterStub } from './workingSetCatalog'
import { createConversationSummaryStub, createConversationSummaryFromMetadata } from './conversationResidency'
import type { Chat, Database, botPreset, character, groupChat } from './database.svelte'
import type { PersistentRevisionReader, PersistentRoot } from './persistentDataStore'
import { canonicalClone, canonicalJson } from './saveCoordinatorHelpers'
import { captureMaterializedCharacter } from './persistentUnitCapture'
import { isWorkingSetCharacterStub } from './workingSetCatalog'

type CompleteCharacter = character | groupChat

export interface LwwWorkingSetBaseline {
    root: PersistentRoot
    presets: botPreset[] | null
    characters: CompleteCharacter[]
}

export function captureLwwWorkingSetBaseline(database: Database, root: PersistentRoot, presets: botPreset[] | null, materialized?: CompleteCharacter[]): LwwWorkingSetBaseline {
    return { root: canonicalClone(root), presets: presets && canonicalClone(presets),
        characters: materialized ?? database.characters.filter((value) => !isWorkingSetCharacterStub(value)).map(captureMaterializedCharacter) }
}

function patchField(live: object, baseline: object, remote: object, field: string): void {
    const current = live as Record<string, unknown>, before = baseline as Record<string, unknown>, next = remote as Record<string, unknown>
    const unchanged = canonicalJson({ value: current[field] }) === canonicalJson({ value: before[field] })
    const write = (value: Record<string, unknown>) => {
        if (!Object.hasOwn(next, field)) delete value[field]
        else Object.defineProperty(value, field, { value: canonicalClone(next[field]), enumerable: true, configurable: true, writable: true })
    }
    if (unchanged) write(current)
    write(before)
}

export async function applyLwwWorkingSetUnits(
    database: Database, baseline: LwwWorkingSetBaseline, store: PersistentRevisionReader, affectedKeys: readonly string[], liveIndex?: ReadonlyMap<string, CompleteCharacter>,
): Promise<LwwWorkingSetBaseline> {
    const charactersById = liveIndex ?? new Map(database.characters.map((value) => [value.chaId,value]))
    const baselineById = new Map(baseline.characters.map((value) => [value.chaId,value]))
    const keys = affectedKeys.map((key) => JSON.parse(key) as string[])
        .sort((a, b) => Number(a[0] === 'root' && ['botPresetsId', 'selectedPersona'].includes(a[1])) - Number(b[0] === 'root' && ['botPresetsId', 'selectedPersona'].includes(b[1])))
    const root = keys.some(([kind]) => ['root', 'variable', 'toggle', 'record', 'order', 'persona', 'preset-protected', 'exists'].includes(kind))
        ? (await store.readRoot()).value : null
    const details = new Map<string, CompleteCharacter | null>()
    for (const [kind, id] of keys) if (kind === 'character' || kind === 'group-members') {
        if (!details.has(id)) {
            const value = await store.readCharacter(id)
            details.set(id, value ? { ...value.value, chats: [] } as CompleteCharacter : null)
        }
    }
    for (const [kind, id, field, metadataField] of keys) {
        if (kind === 'root' && root && persistentRootFields.has(id)) {
            if (id === 'botPresetsId' && typeof root.botPresetsId === 'string') {
                const preset = await store.readPreset(root.botPresetsId)
                if (preset) {
                    const catalog = await store.queryPresets()
                    const index = catalog.items.find((item) => item.id === root.botPresetsId)?.configuredIndex
                    if (index !== undefined) database.botPresets[index] = canonicalClone(preset.value)
                }
            }
            const remote = { ...root } as unknown as Record<string, unknown>
            if (id === 'botPresetsId' && typeof remote[id] === 'string') remote[id] = Math.max(0, database.botPresets.findIndex((value) => value?.['id'] === remote[id]))
            if (id === 'selectedPersona' && typeof remote[id] === 'string') remote[id] = Math.max(0, database.personas?.findIndex((value) => value.id === remote[id]) ?? 0)
            const before = { ...baseline.root } as unknown as Record<string, unknown>
            if (id === 'botPresetsId' && typeof before[id] === 'string') before[id] = Math.max(0, database.botPresets.findIndex((value) => value?.['id'] === before[id]))
            if (id === 'selectedPersona' && typeof before[id] === 'string') before[id] = Math.max(0, database.personas?.findIndex((value) => value.id === before[id]) ?? 0)
            patchField(database, before, remote, id)
            ;(baseline.root as unknown as Record<string, unknown>)[id] = canonicalClone((root as unknown as Record<string, unknown>)[id])
        } else if (kind === 'character' || kind === 'group-members') {
            const live = charactersById.get(id)
            const before = baselineById.get(id)
            const remote = details.get(id)
            if (!live || !before || !remote) continue
            for (const key of kind === 'group-members' ? ['characters', 'characterTalks', 'characterActive'] : [field]) patchField(live, before, remote, key)
        } else if (kind === 'conversation' || kind === 'messages') {
            const liveCharacter = charactersById.get(id)
            const beforeCharacter = baselineById.get(id)
            const live = liveCharacter?.chats.find((value) => value.id === field)
            const before = beforeCharacter?.chats.find((value) => value.id === field)
            if (!live || !before) continue
            if (kind === 'messages') {
                if (!Object.hasOwn(before, 'message') || !Object.prototype.propertyIsEnumerable.call(live, 'message')) continue
                const remote = await store.readConversation(id, field)
                if (remote) {
                    const unchanged = canonicalJson(live.message) === canonicalJson(before.message)
                    patchField(live, before, remote.value, 'message')
                    if (unchanged) liveCharacter!.chats[liveCharacter!.chats.indexOf(live)] = { ...live }
                }
            } else {
                const remote = await store.readConversationMetadata(id, field)
                if (remote) {
                    const unchanged = canonicalJson({value:live[metadataField]}) === canonicalJson({value:before[metadataField]})
                    patchField(live, before, remote.value.conversation, metadataField)
                    if (unchanged) liveCharacter!.chats[liveCharacter!.chats.indexOf(live)] = {...live}
                }
            }
        } else if (kind === 'preset') {
            const live = database.botPresets.find((value) => value?.['id'] === id)
            const before = baseline.presets?.find((value) => value['id'] === id)
            if (!live) continue
            const remote = await store.readPreset(id)
            if (remote) patchField(live, before ?? canonicalClone(live), remote.value, field)
        } else if (root && kind === 'exists' && id === 'persona') {
            const before = baseline.root.personas ?? [], live = database.personas ?? []
            const previous = before.find((value) => value.id === field), current = live.find((value) => value.id === field), remote = root.personas?.find((value) => value.id === field)
            const replace = (values: typeof live) => remote ? [...values.filter((value) => value.id !== field), canonicalClone(remote)] : values.filter((value) => value.id !== field)
            if (canonicalJson(previous ?? null) === canonicalJson(current ?? null)) {
                const selectedId = live[database.selectedPersona]?.id
                database.personas = replace(live)
                database.selectedPersona = Math.max(0, database.personas.findIndex((value) => value.id === selectedId))
            }
            baseline.root.personas = replace(before)
        } else if (root && kind === 'persona') {
            const live = database.personas?.find((value) => value.id === id)
            const before = baseline.root.personas?.find((value) => value.id === id)
            const remote = root.personas?.find((value) => value.id === id)
            if (live && before && remote) patchField(live, before, remote, field)
        } else if (root && kind === 'record') {
            const identity = id === 'plugins' ? 'name' : 'id'
            const live = database as unknown as Record<string, unknown[]>, before = baseline.root as unknown as Record<string, unknown[]>, remote = root as unknown as Record<string, unknown[]>
            const itemId = (value: unknown) => (value as Record<string, unknown>)[identity]
            const original = before[id]?.find((value) => itemId(value) === field)
            const current = live[id]?.find((value) => itemId(value) === field)
            const next = remote[id]?.find((value) => itemId(value) === field)
            const replace = (values: unknown[] = []) => {
                const index = values.findIndex((value) => itemId(value) === field)
                if (next === undefined) { if (index >= 0) values.splice(index, 1) }
                else if (index >= 0) values[index] = canonicalClone(next)
                else values.push(canonicalClone(next))
                return values
            }
            if (canonicalJson(current ?? null) === canonicalJson(original ?? null)) live[id] = replace(live[id])
            before[id] = replace(before[id])
        } else if (root && ['variable', 'toggle', 'preset-protected'].includes(kind)) {
            const rootField = kind === 'preset-protected' ? 'protectedPresetValues' : 'explicitGlobalChatVariables'
            const live = database as unknown as Record<string, Record<string, unknown>>, before = baseline.root as unknown as Record<string, Record<string, unknown>>, remote = root as unknown as Record<string, Record<string, unknown>>
            patchField(live[rootField] ??= {}, before[rootField] ??= {}, remote[rootField] ?? {}, id)
        } else if (kind === 'order' && id === 'conversations') {
            const live = charactersById.get(field)
            const before = baselineById.get(field)
            if (!live || !before) continue
            const ids: string[] = []
            let cursor: string | undefined
            do { const page = await store.queryConversations({ characterId: field, order: 'configured', limit: 128, cursor }); ids.push(...page.items.map((item) => item.id)); cursor = page.nextCursor } while (cursor)
            const order = new Map(ids.map((value,index) => [value,index]))
            const sort = (chats: Chat[]) => [...chats].sort((a,b) => (order.get(a.id) ?? Infinity) - (order.get(b.id) ?? Infinity))
            if (canonicalJson(live.chats.map((value) => value.id)) === canonicalJson(before.chats.map((value) => value.id))) live.chats = sort(live.chats)
            before.chats = sort(before.chats)
            const remoteDetail = await store.readCharacter(field)
            if (remoteDetail) patchField(live, before, remoteDetail.value, 'chatFolders')
        } else if (root && kind === 'order' && ['modules', 'plugins', 'personas', 'loadouts', 'customModels'].includes(id)) {
            const identity = id === 'plugins' ? 'name' : 'id'
            const remote = root as unknown as Record<string, Record<string, unknown>[]>
            const order = new Map((remote[id] ?? []).map((item, index) => [item[identity], index]))
            const sort = (values: Record<string, unknown>[] = []) => [...values].sort((a,b) => (order.get(a[identity]) ?? Infinity) - (order.get(b[identity]) ?? Infinity))
            const live = database as unknown as Record<string, Record<string, unknown>[]>, before = baseline.root as unknown as Record<string, Record<string, unknown>[]>
            if (canonicalJson((live[id] ?? []).map((value) => value[identity])) === canonicalJson((before[id] ?? []).map((value) => value[identity]))) {
                const selectedId = id === 'personas' ? database.personas?.[database.selectedPersona]?.id : undefined
                live[id] = sort(live[id])
                if (id === 'personas') database.selectedPersona = Math.max(0, database.personas?.findIndex((value) => value.id === selectedId) ?? 0)
            }
            before[id] = sort(before[id])
        } else if (root && kind === 'order' && id === 'characters') {
            patchField(database, baseline.root, root, 'characterOrder')
        } else if ((kind === 'order' && id === 'presets') || (kind === 'exists' && id === 'preset')) {
            const selectedId = database.botPresets[database.botPresetsId]?.['id']
            const catalog = await store.queryPresets()
            const byId = new Map(database.botPresets.filter(Boolean).map((value) => [value['id'], value]))
            database.botPresets = catalog.items.map((summary) => byId.get(summary.id) ?? { id: summary.id, name: summary.name, image: summary.image } as unknown as botPreset)
            database.botPresetsId = Math.max(0, database.botPresets.findIndex((value) => value['id'] === selectedId))
            if (baseline.presets) baseline.presets = catalog.items.map((summary) => baseline.presets!.find((value) => value['id'] === summary.id) ?? byId.get(summary.id)!).filter(Boolean)
        } else if (kind === 'exists' && id === 'character') {
            const summary = await store.readCharacterSummary(field)
            if (summary && !charactersById.has(field)) database.characters.push(createCatalogCharacterStub(summary))
            if (!summary) {
                database.characters = database.characters.filter((value) => value.chaId !== field)
                baseline.characters = baseline.characters.filter((value) => value.chaId !== field)
            }
        } else if (kind === 'exists' && id === 'conversation') {
            const live = charactersById.get(field)
            const before = baselineById.get(field)
            const metadata = await store.readConversationMetadata(field, metadataField)
            if (live && before && metadata && !live.chats.some((value) => value.id === metadataField)) {
                const stub = createConversationSummaryStub(createConversationSummaryFromMetadata(field, metadata.value.conversation, live.chats.length, metadata.value.totalMessages, 0))
                live.chats.push(stub)
                before.chats.push({ ...metadata.value.conversation } as Chat)
            }
            if (live && before && !metadata) {
                live.chats = live.chats.filter((value) => value.id !== metadataField)
                before.chats = before.chats.filter((value) => value.id !== metadataField)
            }
        }
    }
    return baseline
}
