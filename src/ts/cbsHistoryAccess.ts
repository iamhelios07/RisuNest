import type { Message } from './storage/database.svelte'

export class CbsHistoryAccessError extends Error {
    constructor(readonly access: string) {
        super(`CBS history was not prepared: ${access}`)
        this.name = 'CbsHistoryAccessError'
    }
}

export interface CbsHistoryAccess {
    readonly length: number
    peek(index: number): Message | undefined
    preparedMessages(): Iterable<Message>
    row(index: number): Message | undefined
    previous(role: Message['role'], before: number, count: number): readonly Message[]
    all(): readonly Message[]
    assertComplete(): void
}

export function createCbsHistoryAccess(input: {
    length: number
    rows: ReadonlyMap<number, Message>
    searches?: ReadonlyMap<string, readonly Message[]>
    complete?: readonly Message[]
}): CbsHistoryAccess {
    let missing: CbsHistoryAccessError | undefined
    function miss(access: string): never {
        missing ??= new CbsHistoryAccessError(access)
        throw missing
    }
    return {
        length: input.length,
        peek(index) { return input.rows.get(index) ?? input.complete?.[index] },
        preparedMessages() { return input.complete ?? input.rows.values() },
        row(index) {
            if (!Number.isInteger(index) || index < 0 || index >= input.length) return undefined
            return input.rows.get(index) ?? input.complete?.[index] ?? miss(`row ${index}`)
        },
        previous(role, before, count) {
            if (before <= 0) return []
            const prepared = input.searches?.get(cbsHistorySearchKey(role, before, count))
            if (prepared) return prepared
            if (input.complete) {
                const result: Message[] = []
                for (let index = Math.min(before, input.length) - 1; index >= 0 && result.length < count; index--) {
                    if (input.complete[index].role === role) result.push(input.complete[index])
                }
                return result
            }
            return miss(`${role} before ${before}, count ${count}`)
        },
        all() { return input.complete ?? miss('complete history') },
        assertComplete() { if (missing) throw missing },
    }
}

export function cbsHistorySearchKey(role: Message['role'], before: number, count: number): string {
    return `${role}:${before}:${count}`
}
