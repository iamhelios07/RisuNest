import {
    normalizeConversationContextInput,
    type ConversationContext,
} from '../conversationContext'
import {
    createConversationPatchLedger,
    normalizeConversationPatchInput,
    type ConversationPatchResult,
} from '../conversationPatch'
import type { ConversationPatchAccess } from '../conversationPatchAccess'
import { linkPluginQueryAbortSignals, type PluginDatabaseAccess } from '../pluginDatabaseAccess'

// Methods for RisuNest's own plugins. They are not part of the public plugin API and
// change together with those plugins.

export interface RisunestPrivateApiDependencies {
    databaseAccess: Pick<PluginDatabaseAccess, 'readConversationContext'>
    patchAccess: ConversationPatchAccess
    /** The periodic `db` permission, the one `getDatabase` asks for. */
    hasDatabasePermission(): Promise<boolean>
    lifetimeSignal: AbortSignal
    now?(): number
}

function throwIfAborted(signal: AbortSignal): void {
    if (!signal.aborted) return
    throw signal.reason ?? new DOMException('The operation was aborted', 'AbortError')
}

function inputSignal(input: unknown): AbortSignal | undefined {
    if (!input || typeof input !== 'object') return undefined
    const signal = (input as { signal?: unknown }).signal
    if (signal === undefined) return undefined
    if (!(signal instanceof AbortSignal)) throw new TypeError('signal must be an AbortSignal')
    return signal
}

export function createRisunestPrivateApi(dependencies: RisunestPrivateApiDependencies) {
    const patchLedger = createConversationPatchLedger(dependencies.now)
    return {
        async readConversationContext(input?: unknown): Promise<ConversationContext | null> {
            const request = normalizeConversationContextInput(input)
            const linked = linkPluginQueryAbortSignals(inputSignal(input), dependencies.lifetimeSignal)
            try {
                throwIfAborted(linked.signal)
                const allowPrivate = request.include.persona || request.include.globals
                    ? await dependencies.hasDatabasePermission()
                    : false
                throwIfAborted(linked.signal)
                const result = await dependencies.databaseAccess.readConversationContext(request, {
                    allowPrivate,
                    signal: linked.signal,
                })
                throwIfAborted(linked.signal)
                return result
            } finally {
                linked.dispose()
            }
        },
        async patchConversation(input?: unknown): Promise<ConversationPatchResult> {
            const request = normalizeConversationPatchInput(input)
            const signal = inputSignal(input)
            return patchLedger.run(request.mutationId, async () => {
                const linked = linkPluginQueryAbortSignals(signal, dependencies.lifetimeSignal)
                try {
                    throwIfAborted(linked.signal)
                    return await dependencies.patchAccess.patchConversation(request, linked.signal)
                } finally {
                    linked.dispose()
                }
            })
        },
    }
}
