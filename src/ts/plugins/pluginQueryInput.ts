import type { ConversationWindowQuery } from '../storage/persistentDataStore'

export const PLUGIN_SUMMARY_QUERY_DEFAULT_LIMIT = 50
export const PLUGIN_SUMMARY_QUERY_MAX_LIMIT = 100
export const PLUGIN_MESSAGE_QUERY_DEFAULT_LIMIT = 128
export const PLUGIN_MESSAGE_QUERY_MAX_LIMIT = 128

export interface PluginMessageWindowInput {
    startIndex?: number
    limit?: number
    anchorMessageId?: string
    before?: number
    after?: number
}

export function positiveLimit(value: number | undefined, defaultValue: number, maximum: number): number {
    const limit = value ?? defaultValue
    if (!Number.isSafeInteger(limit) || limit <= 0) {
        throw new RangeError('Query limit must be a positive safe integer')
    }
    return Math.min(limit, maximum)
}

export function requiredId(value: string, name: string): void {
    if (typeof value !== 'string' || value.trim().length === 0) {
        throw new RangeError(`${name} must be a nonempty string`)
    }
}

function nonnegativeWindow(value: number | undefined, name: string): number {
    const size = value ?? 0
    if (!Number.isSafeInteger(size) || size < 0) {
        throw new RangeError(`${name} must be a nonnegative safe integer`)
    }
    return size
}

export type PluginMessageWindow = Omit<ConversationWindowQuery, 'characterId' | 'conversationId'>

/** Validates one of the three plugin message window modes and returns its store query fields. */
export function pluginMessageWindow(input: PluginMessageWindowInput): PluginMessageWindow {
    const ranged = input.startIndex !== undefined
    const anchored = input.anchorMessageId !== undefined
    if (ranged) {
        if (!Number.isSafeInteger(input.startIndex) || input.startIndex! < 0) {
            throw new RangeError(
                'Message range startIndex must be a nonnegative safe integer',
            )
        }
        if (input.limit === undefined) {
            throw new RangeError('Absolute message ranges require limit')
        }
        if (
            anchored ||
            input.before !== undefined ||
            input.after !== undefined
        ) {
            throw new RangeError('Absolute message ranges cannot include anchor options')
        }
        return {
            startIndex: input.startIndex,
            limit: positiveLimit(
                input.limit,
                PLUGIN_MESSAGE_QUERY_DEFAULT_LIMIT,
                PLUGIN_MESSAGE_QUERY_MAX_LIMIT,
            ),
        }
    }
    if (anchored) {
        requiredId(input.anchorMessageId!, 'anchorMessageId')
        if (input.limit !== undefined) {
            throw new RangeError('Anchored message queries cannot include limit')
        }
        const before = nonnegativeWindow(input.before, 'before')
        const after = nonnegativeWindow(input.after, 'after')
        if (before + 1 + after > PLUGIN_MESSAGE_QUERY_MAX_LIMIT) {
            throw new RangeError('Anchored message window exceeds the maximum size')
        }
        return {
            anchorMessageId: input.anchorMessageId,
            before,
            after,
        }
    }
    if (input.before !== undefined || input.after !== undefined) {
        throw new RangeError('Message window offsets require anchorMessageId')
    }
    return {
        limit: positiveLimit(
            input.limit,
            PLUGIN_MESSAGE_QUERY_DEFAULT_LIMIT,
            PLUGIN_MESSAGE_QUERY_MAX_LIMIT,
        ),
    }
}
