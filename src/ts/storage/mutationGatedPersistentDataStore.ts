import type {
    AssetAlias,
    AssetAliasIdentity,
    AssetAliasKind,
    AssetAliasListQuery,
    AssetOwnerLocator,
    CharacterPage,
    CharacterQuery,
    ConversationPage,
    ConversationQuery,
    ConversationWindow,
    ConversationWindowQuery,
    DataRevision,
    PersistentDataStore,
    PersistentRevisionLease,
    Versioned,
    WorkingSetCommit,
    CharacterDetail,
} from './persistentDataStore'
import type { StorageMutationGate } from './storageMutationGate'

export function createMutationGatedPersistentDataStore(
    store: PersistentDataStore,
    gate: StorageMutationGate,
): PersistentDataStore {
    const gated = {
        open: () => store.open(),
        lwwReadOutbox: store.lwwReadOutbox ? (request) => store.lwwReadOutbox!(request) : undefined,
        lwwClockState: store.lwwClockState ? (request) => store.lwwClockState!(request) : undefined,
        lwwAckOutbox: store.lwwAckOutbox ? (request) => gate.runWrite(() => store.lwwAckOutbox!(request)) : undefined,
        lwwRetryUnpublished: store.lwwRetryUnpublished ? (request) => gate.runWrite(() => store.lwwRetryUnpublished!(request)) : undefined,
        lwwCommitReplacement: store.lwwCommitReplacement ? (request) => gate.runTransition(() => store.lwwCommitReplacement!(request)) : undefined,
        lwwStageReceive: store.lwwStageReceive ? (request) => gate.runWrite(() => store.lwwStageReceive!(request)) : undefined,
        lwwApplyReceive: store.lwwApplyReceive ? (request) => gate.runWrite(() => store.lwwApplyReceive!(request)) : undefined,
        lwwFinishReceive: store.lwwFinishReceive ? (request) => gate.runWrite(() => store.lwwFinishReceive!(request)) : undefined,
        lwwDrainDeferred: store.lwwDrainDeferred ? (request) => gate.runWrite(() => store.lwwDrainDeferred!(request)) : undefined,

        readRoot: () => store.readRoot(),
        queryPresets: () => store.queryPresets(),
        readPreset: (id: string) => store.readPreset(id),
        queryCharacters: (input: CharacterQuery): Promise<CharacterPage> =>
            store.queryCharacters(input),
        readCharacterSummary: (id: string) => store.readCharacterSummary(id),
        readCharacter: (id: string): Promise<Versioned<CharacterDetail> | null> =>
            store.readCharacter(id),
        queryConversations: (input: ConversationQuery): Promise<ConversationPage> =>
            store.queryConversations(input),
        readConversation: (characterId, conversationId) =>
            store.readConversation(characterId, conversationId),
        readConversationMetadata: (characterId, conversationId) =>
            store.readConversationMetadata(characterId, conversationId),
        readConversationWindow: (
            input: ConversationWindowQuery,
        ): Promise<Versioned<ConversationWindow> | null> => store.readConversationWindow(input),
        queryPluginStorage: () => store.queryPluginStorage(),
        readPluginStorage: (owner: string, key: string) => store.readPluginStorage(owner, key),
        listPluginStorage: () => store.listPluginStorage(),
        readPluginStorageValues: (query) => store.readPluginStorageValues(query),
        readConversationMessageMetadataWindow: store.readConversationMessageMetadataWindow
            ? (input) => store.readConversationMessageMetadataWindow!(input)
            : undefined,
        commitWorkingSetChangeCursor: store.commitWorkingSetChangeCursor
            ? (revision: DataRevision) => store.commitWorkingSetChangeCursor!(revision)
            : undefined,
        readAssetAlias: (identity: AssetAliasIdentity) => store.readAssetAlias(identity),
        readAssetAliasesByKeys: (kind: AssetAliasKind, keys: string[]) =>
            store.readAssetAliasesByKeys(kind, keys),
        listAssetAliases: (input: AssetAliasListQuery) => store.listAssetAliases(input),
        readAssetOwnerHead: (owner: AssetOwnerLocator) => store.readAssetOwnerHead(owner),
        commitAssetAlias: (alias: AssetAlias, expectedRevision: DataRevision) =>
            gate.runWrite(() => store.commitAssetAlias(alias, expectedRevision)),
        deleteAssetAlias: (identity: AssetAliasIdentity, expectedRevision: DataRevision) =>
            gate.runWrite(() => store.deleteAssetAlias(identity, expectedRevision)),
        commit: (input: WorkingSetCommit) => gate.runWrite(() => store.commit(input)),
        archivePreview: (characterId: string) => store.archivePreview(characterId),
        archiveCharacter: (
            characterId: string,
            expectedRevision: DataRevision,
            signal?: AbortSignal,
        ) => gate.runWrite(() => store.archiveCharacter(characterId, expectedRevision, signal)),
        restoreCharacter: (
            characterId: string,
            expectedRevision: DataRevision,
            signal?: AbortSignal,
        ) => gate.runWrite(() => store.restoreCharacter(characterId, expectedRevision, signal)),
        replaceFromDatabase: (...args: Parameters<PersistentDataStore['replaceFromDatabase']>) =>
            gate.runTransition(() => store.replaceFromDatabase(...args)),
        materializeDatabase: (revision?: DataRevision) => store.materializeDatabase(revision),
        acquireRevision: (revision: DataRevision): Promise<PersistentRevisionLease> =>
            store.acquireRevision(revision),
    } satisfies { [K in keyof PersistentDataStore]-?: PersistentDataStore[K] | undefined }
    return gated
}
