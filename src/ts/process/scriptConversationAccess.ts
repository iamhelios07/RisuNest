import {
    ConversationSessionStaleError,
    requireCurrentConversationSession,
    type ActiveConversationSession,
} from '../storage/activeConversationSession'
import type { Chat, Database } from '../storage/database.svelte'
import { peekActiveConversationSession } from '../storage/persistentDataRuntime.svelte'
import { parseKeyValue } from '../util'
import {
    createConversationOperationContext,
    type ConversationCommitObserver,
    type ConversationOperationContext,
} from './conversationOperationContext'

export interface ScriptConversationAccessOptions {
    session: ActiveConversationSession
    /** The live conversation object the session owns. */
    chat: Chat
    /** Resolves character and template default variables. */
    database: Database
    characterId: string
    onCommitted?: ConversationCommitObserver
    /** Display listeners commit with display origin, which keeps rendered rows valid. */
    origin?: 'display'
    /**
     * Called with the changed keys after each variable-only commit. Rendered rows keep
     * their output across those commits; a message operation re-renders them itself.
     */
    onVariablesCommitted?: (keys: readonly string[]) => void
}

/**
 * One script invocation's view of the selected conversation. Chat variables are read
 * from the live conversation and buffered on write; messages are read live and copied
 * only when the script first edits them.
 */
export class ScriptConversationAccess {
    private readonly pendingVariables = new Map<string, string>()
    private operation: ConversationOperationContext | null = null
    private firstReadMessageVersion: number | null = null
    private failure: { error: unknown } | null = null
    private defaultVariables: Map<string, string> | null | undefined
    private readonly precedingGenerationVersion: number

    constructor(private readonly options: ScriptConversationAccessOptions) {
        this.precedingGenerationVersion = options.session.generationInvalidationVersion
    }

    get session(): ActiveConversationSession {
        return this.options.session
    }

    get characterId(): string {
        return this.options.characterId
    }

    /** The live conversation object the session owns. */
    get liveChat(): Chat {
        return this.options.chat
    }

    get origin(): 'display' | undefined {
        return this.options.origin
    }

    get conversationOperation(): ConversationOperationContext | null {
        return this.operation
    }

    get hasPendingVariables(): boolean {
        return this.pendingVariables.size > 0
    }

    getVariable(key: string): string {
        const pending = this.pendingVariables.get(key)
        if (pending !== undefined) return pending
        const defaults = this.readDefaultVariables()
        if (defaults === null) return 'null'
        this.requireCurrent()
        const state = this.options.session.readChatVariable(key)
        if (state === undefined || state === null) return defaults.get(key) ?? 'null'
        return String(state)
    }

    /** Buffers a variable write. Returns whether it changes the stored value. */
    setVariable(key: string, value: string): boolean {
        const current = this.pendingVariables.has(key)
            ? this.pendingVariables.get(key)
            : this.options.session.readChatVariable(key)
        if (current === value) return false
        this.pendingVariables.set(key, value)
        return true
    }

    /** The conversation to read: live until the script edits it, then its copy. */
    readChat(): Chat {
        if (this.operation) return this.operation.chat
        this.requireCurrent()
        this.firstReadMessageVersion ??= this.options.session.messageVersion
        return this.options.chat
    }

    /** The conversation this access currently exposes, without recording a read. */
    currentChat(): Chat {
        return this.operation?.chat ?? this.options.chat
    }

    /** Opens the message operation for an edit, unless messages changed since the first read. */
    writeOperation(): ConversationOperationContext {
        if (this.operation) return this.operation
        this.requireCurrent()
        const session = this.options.session
        if (
            this.firstReadMessageVersion !== null &&
            session.messageVersion !== this.firstReadMessageVersion
        ) {
            this.fail(new ConversationSessionStaleError(
                this.firstReadMessageVersion,
                session.messageVersion,
            ))
        }
        this.operation = createConversationOperationContext(
            session,
            this.options.chat,
            this.options.onCommitted,
            { variablesFollowSession: true },
        )
        return this.operation
    }

    writeChat(): Chat {
        return this.writeOperation().chat
    }

    databaseView(database: Database): Database {
        return this.operation?.createDatabaseView(database) ?? database
    }

    /** Commits buffered variables now, last writer wins. Returns the changed keys. */
    flushVariables(): readonly string[] {
        if (this.pendingVariables.size === 0) return []
        this.requireCurrent()
        const changes = new Map(this.pendingVariables)
        this.pendingVariables.clear()
        const session = this.options.session
        const baseVersion = session.version
        const keys = session.updateChatVariables(changes, {
            ...(this.options.origin ? { origin: this.options.origin } : {}),
        })
        if (keys.length === 0) return keys
        const version = session.version
        const chat = this.options.chat
        // The same receipt an operation reports, so a generation waiting on this
        // conversation adopts the commit instead of treating it as a foreign edit.
        this.options.onCommitted?.({
            follows: (current, previousVersion, target) =>
                current === session &&
                previousVersion !== null &&
                previousVersion >= this.precedingGenerationVersion &&
                previousVersion <= baseVersion &&
                target === chat &&
                session.isActive &&
                session.version === version &&
                session.materializeCompatibilityArray() === target.message,
        })
        this.options.onVariablesCommitted?.(keys)
        return keys
    }

    /**
     * Commits everything the invocation changed. Variables and message edits commit
     * together. If the edits are stale, or the script hit a stale or closed conversation,
     * the variables still commit and the error is rethrown.
     */
    commit(): void {
        this.finish((operation) => operation.commit(peekActiveConversationSession(), {
            ...(this.options.origin ? { origin: this.options.origin } : {}),
            variables: new Map(this.pendingVariables),
        }))
    }

    /** Commits variables and conversation metadata, dropping message edits. */
    commitWithoutMessages(): void {
        this.finish((operation) => operation.commitMetadata(
            peekActiveConversationSession(),
            new Map(this.pendingVariables),
        ))
    }

    private finish(commitOperation: (operation: ConversationOperationContext) => void): void {
        const operation = this.operation
        const failure = this.failure
        this.operation = null
        this.failure = null
        this.firstReadMessageVersion = null
        if (failure) {
            operation?.release()
            this.commitVariablesAfterFailure()
            throw failure.error
        }
        if (!operation) {
            this.requireCurrent()
            this.flushVariables()
            return
        }
        try {
            commitOperation(operation)
            this.pendingVariables.clear()
        } catch (error) {
            this.commitVariablesAfterFailure()
            throw error
        }
    }

    private commitVariablesAfterFailure(): void {
        try {
            this.flushVariables()
        } catch {
            this.pendingVariables.clear()
        }
    }

    /** Errors the script sees are swallowed by the script host, so the commit rethrows them. */
    private fail(error: unknown): never {
        this.failure ??= { error }
        throw error
    }

    private requireCurrent(): void {
        try {
            requireCurrentConversationSession(this.options.session, peekActiveConversationSession())
        } catch (error) {
            this.fail(error)
        }
    }

    private readDefaultVariables(): Map<string, string> | null {
        if (this.defaultVariables !== undefined) return this.defaultVariables
        const database = this.options.database
        const character = database.characters?.find(
            (candidate) => candidate.chaId === this.options.characterId,
        )
        if (!character) return (this.defaultVariables = null)
        const defaults = new Map<string, string>()
        for (const [key, value] of parseKeyValue(character.defaultVariables).concat(
            parseKeyValue(database.templateDefaultVariables),
        )) {
            if (!defaults.has(key)) defaults.set(key, value)
        }
        return (this.defaultVariables = defaults)
    }
}

/** An access for the selected conversation, or null when the session does not own `chat`. */
export function createScriptConversationAccess(
    chat: Chat | null | undefined,
    database: Database,
    options: Pick<
        ScriptConversationAccessOptions,
        'onCommitted' | 'origin' | 'onVariablesCommitted'
    > = {},
): ScriptConversationAccess | null {
    const session = peekActiveConversationSession()
    if (!chat || !session?.isActive || session.materializeCompatibilityArray() !== chat.message) {
        return null
    }
    return new ScriptConversationAccess({
        session,
        chat,
        database,
        characterId: session.characterId,
        ...options,
    })
}
