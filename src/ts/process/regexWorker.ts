import type {
    RegexWorkerRequest,
    RegexWorkerResponse,
} from './regexWorkerClient'
import { executeRegexOperation } from './regexOperations'

interface CompiledEntry {
    sourceIndex: number
    pattern: string
    replacement: string
    regex?: RegExp
    compileError?: string
    host?: boolean
    conditional?: boolean
}

function errorMessage(error: unknown): string {
    return error instanceof Error ? error.message : String(error)
}

export function createRegexWorkerMessageHandler(
    respond: (response: RegexWorkerResponse) => void,
): (request: RegexWorkerRequest) => void {
    let registeredRevision: number | undefined
    let registeredEntries: CompiledEntry[] | undefined
    let retainedInput: string | undefined

    return (request) => {
        if (request.type === 'release') {
            retainedInput = undefined
            return
        }
        if (request.type === 'operation') {
            try {
                respond({ type: 'operationResult', id: request.id, value: executeRegexOperation(request.operation) })
            } catch (error) {
                respond({ type: 'error', id: request.id, message: errorMessage(error) })
            }
            return
        }
        if (request.type === 'register') {
            retainedInput = undefined
            registeredRevision = request.revision
            registeredEntries = request.entries.map(([
                sourceIndex,
                pattern,
                replacement,
                flags,
                host,
                conditional,
            ]) => {
                const entry: CompiledEntry = { sourceIndex, pattern, replacement, host, conditional }
                if (pattern !== '' && !host) {
                    try {
                        entry.regex = new RegExp(pattern, flags)
                    }
                    catch (error) {
                        entry.compileError = errorMessage(error)
                    }
                }
                return entry
            })
            return
        }

        if (registeredRevision !== request.revision || registeredEntries === undefined) {
            respond({
                type: 'error',
                id: request.id,
                message: `Regex plan revision ${request.revision} is not registered`,
            })
            return
        }

        if (request.reuseInput && retainedInput === undefined) {
            respond({ type: 'error', id: request.id, message: 'Regex continuation input is unavailable' })
            return
        }
        let data = request.reuseInput ? retainedInput! : request.input
        const errors: [sourceIndex: number, message: string][] = []
        const steps: { index: number, matched: boolean }[] = []
        let nextIndex = request.continuation?.start ?? 0
        const end = request.continuation?.end ?? registeredEntries.length
        for (; nextIndex < end; nextIndex++) {
            const entry = registeredEntries[nextIndex]
            if (request.continuation && entry.host) break
            if (entry.pattern === '') {
                continue
            }
            try {
                if (entry.compileError !== undefined) {
                    throw new Error(entry.compileError)
                }
                if (entry.regex === undefined) {
                    throw new Error('Regex Worker entry was not compiled')
                }
                entry.regex.lastIndex = 0
                const matched = entry.regex.test(data)
                entry.regex.lastIndex = 0
                const replaced = matched ? data.replace(entry.regex, entry.replacement) : data
                steps.push({ index: nextIndex, matched })
                if (request.continuation && replaced !== data) {
                    data = replaced
                    nextIndex++
                    break
                }
                data = replaced
            }
            catch (error) {
                errors.push([entry.sourceIndex, errorMessage(error)])
            }
        }

        retainedInput = request.continuation ? data : undefined
        respond({ type: 'result', id: request.id, data, errors,
            ...(request.continuation ? { steps, nextIndex } : {}),
        })
    }
}

const workerScope = globalThis as typeof globalThis & {
    document?: unknown
    postMessage?: (message: RegexWorkerResponse) => void
    addEventListener?: (type: 'message', listener: EventListener) => void
}

if (workerScope.document === undefined
    && typeof workerScope.postMessage === 'function'
    && typeof workerScope.addEventListener === 'function') {
    const handleRequest = createRegexWorkerMessageHandler((response) => {
        workerScope.postMessage!(response)
    })
    workerScope.postMessage({ type: 'ready' })
    workerScope.addEventListener('message', ((event: MessageEvent<RegexWorkerRequest>) => {
        handleRequest(event.data)
    }) as EventListener)
}
