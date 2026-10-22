import { Mutex } from '../mutex'
import { ByteBudgetLru } from '../util/byteBudgetLru'
import type { RegexExecutionPlan, RegexExecutionPlanEntry, RegexExecutionResult } from './regexExecutionPlan'
import { executeRegexOperation, unmatchedRegexOperation, type RegexOperation, type RegexOperationResults } from './regexOperations'
import { DEFAULT_REGEX_WORKER_TIMEOUT_MS, getSharedRegexWorkerClient, isRegexWorkerAvailable, RegexExecutionTimeoutError, RegexWorkerResetError, type RegexWorkerClient } from './regexWorkerClient'
import { createRegexTimeoutScope, type RegexTimeoutScope } from './regexTimeoutScope'

export const DEFAULT_REGEX_INVOCATION_TIMEOUT_MS = 30_000
export class RegexRuleSkippedError extends Error {}

// Hold the queue through isolation so other rows cannot retry the culprit first.
const executionMutex = new Mutex()
export class RegexTimeoutMemory {
    private readonly entries: ByteBudgetLru<string, string[]>
    constructor(maxBytes = 2 * 1024 * 1024, maxEntries = 128) {
        this.entries = new ByteBudgetLru(maxBytes,
            (key, inputs) => 2 * (key.length + inputs.reduce((size, input) => size + input.length, 0)), maxEntries)
    }
    has(key: string, input: string): boolean { return this.entries.get(key)?.includes(input) === true }
    hasRule(key: string): boolean { return this.entries.get(key) !== undefined }
    remember(key: string, input: string): void {
        this.entries.set(key, [...(this.entries.get(key) ?? []).filter((text) => text !== input).slice(-3), input])
    }
}
const recentTimeouts = new RegexTimeoutMemory()

async function runExclusive<T>(signal: AbortSignal | undefined, action: () => Promise<T>): Promise<T> {
    signal?.throwIfAborted()
    const acquisition = executionMutex.acquire()
    let abort: (() => void) | undefined
    let release: (() => void) | undefined
    try {
        release = signal ? await Promise.race([acquisition, new Promise<never>((_, reject) => {
            abort = () => reject(signal.reason ?? new DOMException('The operation was aborted', 'AbortError'))
            signal.addEventListener('abort', abort, { once: true })
        })]) : await acquisition
        signal?.throwIfAborted()
        return await action()
    } finally {
        if (abort) signal?.removeEventListener('abort', abort)
        if (release) release()
        else void acquisition.then((release) => release())
    }
}

export interface RegexExecutorOptions {
    signal?: AbortSignal
    scope?: RegexTimeoutScope
    worker?: boolean
    client?: RegexWorkerClient
    requestTimeoutMs?: number
    invocationTimeoutMs?: number
    onTimeout?: () => void
    memory?: RegexTimeoutMemory
}

export function regexRuleKey(entry: RegexExecutionPlanEntry): string {
    return JSON.stringify([entry.script.type, entry.pattern, entry.flags, entry.replacement, entry.actions])
}

export class RegexExecutor {
    lastOperationSkipped = false
    private readonly scope: RegexTimeoutScope
    private timedOutMs = 0
    private worker: boolean
    private readonly client: RegexWorkerClient

    constructor(private readonly options: RegexExecutorOptions = {}) {
        this.scope = options.scope ?? createRegexTimeoutScope()
        this.worker = options.worker ?? isRegexWorkerAvailable()
        this.client = options.client ?? getSharedRegexWorkerClient()
    }

    private skipped(key: string, input: string): boolean {
        return this.timedOutMs >= (this.options.invocationTimeoutMs ?? DEFAULT_REGEX_INVOCATION_TIMEOUT_MS)
            || this.scope.skipped.has(key) || (this.options.memory ?? recentTimeouts).has(key, input)
    }

    private timeout(key: string, input: string): void {
        this.scope.skipped.add(key)
        const memory = this.options.memory ?? recentTimeouts
        memory.remember(key, input)
        if (!this.scope.noticeShown) {
            this.scope.noticeShown = true
            this.options.onTimeout?.()
        }
    }

    private async request<T>(run: (timeoutMs: number) => Promise<T>, fallback: () => T): Promise<T> {
        this.options.signal?.throwIfAborted()
        if (!this.worker) return fallback()
        const timeoutMs = Math.min(this.options.requestTimeoutMs ?? DEFAULT_REGEX_WORKER_TIMEOUT_MS,
            (this.options.invocationTimeoutMs ?? DEFAULT_REGEX_INVOCATION_TIMEOUT_MS) - this.timedOutMs)
        let resetRetry = false
        try {
            try {
                return await run(timeoutMs)
            } catch (error) {
                if (!(error instanceof RegexWorkerResetError) || this.options.signal?.aborted) throw error
                resetRetry = true
                return await run(timeoutMs)
            }
        } catch (error) {
            this.options.signal?.throwIfAborted()
            if (error instanceof RegexExecutionTimeoutError) {
                this.timedOutMs += timeoutMs
                throw error
            }
            if (resetRetry || error instanceof RegexWorkerResetError) throw error
            // Only unavailable Workers fall back. Operation errors are delivered as
            // normal regex errors and must not execute the expression twice.
            if (error instanceof Error && error.name === 'RegexWorkerOperationError') throw error
            this.worker = false
            return fallback()
        }
    }

    async operation<K extends keyof RegexOperationResults>(operation: RegexOperation<K>, key = JSON.stringify([operation.pattern, operation.flags])): Promise<RegexOperationResults[K]> {
        return runExclusive(this.options.signal, () => this.operationUnlocked(operation, key))
    }

    private async operationUnlocked<K extends keyof RegexOperationResults>(operation: RegexOperation<K>, key: string): Promise<RegexOperationResults[K]> {
        this.options.signal?.throwIfAborted()
        this.lastOperationSkipped = this.skipped(key, operation.input)
        if (this.lastOperationSkipped) return unmatchedRegexOperation(operation)
        try {
            return await this.request(
                (timeoutMs) => this.client.operate(operation, { timeoutMs, signal: this.options.signal }),
                () => executeRegexOperation(operation),
            )
        } catch (error) {
            if (!(error instanceof RegexExecutionTimeoutError)) throw error
            this.timeout(key, operation.input)
            this.lastOperationSkipped = true
            return unmatchedRegexOperation(operation)
        }
    }

    async scriptOperation<K extends keyof RegexOperationResults>(operation: RegexOperation<K>, key: string): Promise<RegexOperationResults[K]> {
        if (this.skipped(key, operation.input)) throw new RegexRuleSkippedError()
        const result = await this.operationUnlocked(operation, key)
        if (this.skipped(key, operation.input)) throw new RegexRuleSkippedError()
        return result
    }

    async execute(plan: RegexExecutionPlan, input: string, parse: (text: string) => string | Promise<string>,
        host: (entry: RegexExecutionPlanEntry, input: string, executor: RegexExecutor) => Promise<string>): Promise<RegexExecutionResult> {
        return runExclusive(this.options.signal, async () => {
            let data = input
            let index = 0
            let isolateUntil = 0
            const errors: RegexExecutionResult['errors'] = []
            const keys = plan.entries.map(regexRuleKey)
            const memory = this.options.memory ?? recentTimeouts
            const skipCandidates = keys.flatMap((key, index) =>
                this.scope.skipped.has(key) || memory.hasRule(key) ? [index] : [])
            try {
                while (index < plan.entries.length) {
                    this.options.signal?.throwIfAborted()
                    const entry = plan.entries[index]
                    if (entry.pattern === '' || this.skipped(keys[index], data)) { index++; continue }
                    if (entry.requiresHostExecution || entry.dynamicPattern || !this.worker) {
                        data = await host(entry, data, this)
                        index++
                        continue
                    }
                    let end = index < isolateUntil ? index + 1 : plan.entries.length
                    for (const i of skipCandidates) {
                        if (i > index && i < end && this.skipped(keys[i], data)) { end = i; break }
                    }
                    let result: RegexExecutionResult
                    try {
                        result = await this.request<RegexExecutionResult>((timeoutMs) => this.client.execute(plan, data, {
                            timeoutMs, signal: this.options.signal, continuation: { start: index, end },
                        }), () => ({ data, errors: [], nextIndex: index, steps: [] }))
                    } catch (error) {
                        if (!(error instanceof RegexExecutionTimeoutError)) throw error
                        if (index < isolateUntil) {
                            this.timeout(keys[index], data)
                            index++
                        } else {
                            isolateUntil = end
                        }
                        continue
                    }
                    this.options.signal?.throwIfAborted()
                    if (!this.worker) continue
                    const next = result.nextIndex!
                    let replayChanged = false
                    for (const step of result.steps!) {
                        const current = plan.entries[step.index]
                        const before = step.index === next - 1 ? result.data : data
                        if (current.actions.length === 0 || step.matched) {
                            data = await parse(before)
                            if (data !== before) {
                                index = step.index + 1
                                replayChanged = true
                                break
                            }
                        } else data = before
                    }
                    // Speculative failures after a parse changed the input are discarded too.
                    errors.push(...result.errors.filter((error) => !replayChanged
                        || plan.entries.findIndex((candidate) => candidate.sourceIndex === error.sourceIndex) < index))
                    if (!replayChanged) index = next
                }
                return { data, errors }
            } finally {
                this.client.releaseContinuation()
            }
        })
    }
}
