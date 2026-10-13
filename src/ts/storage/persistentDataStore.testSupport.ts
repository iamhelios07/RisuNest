import type { PersistentRevisionLease } from './persistentDataStore'

type OptionalKey<T> = { [K in keyof T]-?: {} extends Pick<T, K> ? K : never }[keyof T]
type MemberKind<T> = {
    [K in keyof T]-?: K extends OptionalKey<T>
        ? 'optional'
        : T[K] extends (...args: never[]) => unknown ? 'method' : 'value'
}

// Every lease member, so a new member fails `pnpm check` here instead of in each fake.
const leaseMembers = {
    revision: 'value',
    readRoot: 'method',
    queryPresets: 'method',
    readPreset: 'method',
    queryCharacters: 'method',
    readCharacterSummary: 'method',
    readCharacter: 'method',
    queryConversations: 'method',
    readConversation: 'method',
    readConversationMetadata: 'method',
    readConversationWindow: 'method',
    readConversationMessageMetadataWindow: 'optional',
    readPluginStorageValues: 'method',
    queryPluginStorage: 'method',
    readPluginStorage: 'method',
    readAssetAlias: 'method',
    readAssetAliasesByKeys: 'method',
    listAssetAliases: 'method',
    readAssetOwnerHead: 'method',
    readWorkingSetChangeWindow: 'optional',
    readWorkingSetChangePage: 'optional',
    release: 'method',
} satisfies MemberKind<PersistentRevisionLease>

/**
 * A revision lease built from `members`, plus `extra` own keys such as the native lease handle.
 * An omitted method throws `unexpected call: <name>`, an omitted `revision` throws when read,
 * and an omitted optional member stays absent, as on a store without that capability.
 */
export function createLeaseStub<Extra extends object = {}>(
    members: Partial<PersistentRevisionLease>,
    extra?: Extra,
): PersistentRevisionLease & Extra {
    const lease: object = {}
    for (const [name, kind] of Object.entries(leaseMembers)) {
        if (kind === 'optional' || Object.hasOwn(members, name)) continue
        if (kind === 'method') {
            Object.assign(lease, { [name]: () => { throw new Error(`unexpected call: ${name}`) } })
        } else {
            Object.defineProperty(lease, name, {
                configurable: true,
                get() { throw new Error(`unexpected read: ${name}`) },
            })
        }
    }
    Object.defineProperties(lease, Object.getOwnPropertyDescriptors(members))
    return Object.defineProperties(lease, Object.getOwnPropertyDescriptors(extra ?? {})) as PersistentRevisionLease & Extra
}
