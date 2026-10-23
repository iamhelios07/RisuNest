import { LuaWorkerStrings, decodeLuaFailure, encodeLuaFailure, type LuaWorkerCommand, type LuaWorkerEvent } from './luaWorkerProtocol'
import { LuaWorkerTransport } from './luaWorkerTransport'

type HostBoundary = { run<T>(callback: () => T): T; flush(): void }

export class LuaWorkerExecutor {
    private readonly transport: LuaWorkerTransport
    private readonly engine: number
    private readonly callbacks = new Map<string, Function>()
    private sequence = 0
    private promise = 0
    private pending?: { sequence: number; resolve: (value: unknown) => void; reject: (error: unknown) => void }
    private closed = false
    private removeAbort?: () => void
    private readonly strings = new LuaWorkerStrings()

    constructor(private readonly timeout: number, private readonly boundary?: HostBoundary, createWorker?: () => Worker) {
        const connection = LuaWorkerTransport.connect({ receive: event => this.receive(event), fail: error => this.fail(error) }, createWorker)
        this.transport = connection.transport
        this.engine = connection.engine
    }

    register(name: string, callback: Function): void { this.callbacks.set(name, callback) }

    initialize(source: string, json: string, signal?: AbortSignal): Promise<unknown> {
        return this.command({ kind: 'initialize', source, json, names: [...this.callbacks.keys()], timeout: this.timeout, sequence: ++this.sequence }, signal)
    }

    invoke(name: string, args: unknown[], signal?: AbortSignal): Promise<unknown> {
        return this.command({ kind: 'invoke', name, args, sequence: ++this.sequence }, signal)
    }

    private command(message: Extract<LuaWorkerCommand, { sequence: number }>, signal?: AbortSignal): Promise<unknown> {
        if (this.closed) return Promise.reject(new Error('Lua executor closed'))
        if (this.pending) return Promise.reject(new Error('Concurrent Lua invocation'))
        if (signal?.aborted) return Promise.reject(signal.reason)
        return new Promise((resolve, reject) => {
            this.strings.clear()
            this.pending = { sequence: message.sequence, resolve, reject }
            if (signal) {
                const abort = () => this.fail(signal.reason instanceof Error ? signal.reason : new Error('Lua invocation aborted'))
                signal.addEventListener('abort', abort, { once: true })
                this.removeAbort = () => signal.removeEventListener('abort', abort)
            }
            this.transport.begin(this.engine, message.kind === 'initialize' ? 5_000 : this.timeout)
            this.send(message)
        })
    }

    private send(message: LuaWorkerCommand): void {
        if (this.closed) return
        this.transport.send(this.engine, message)
    }

    private receive(message: LuaWorkerEvent): void {
        if (this.closed) return
        if (message.kind === 'heartbeat' || message.kind === 'disposed') return
        const pending = this.pending
        if (!pending || pending.sequence !== message.sequence) return
        if (message.kind === 'result') {
            this.boundary?.flush()
            if (message.fatal) { this.fail(new Error(message.error || 'Lua worker failed')); return }
            this.transport.end(this.engine)
            this.removeAbort?.()
            this.removeAbort = undefined
            this.pending = undefined
            this.strings.clear()
            if (message.failure !== undefined) pending.reject(decodeLuaFailure(message.failure))
            else if (message.error !== undefined) pending.reject(new Error(message.error))
            else pending.resolve(message.value)
            return
        }
        try {
            const callback = this.callbacks.get(message.name)
            if (!callback) throw new Error('Unknown Lua host API')
            const value = this.boundary ? this.boundary.run(() => callback(...message.args)) : callback(...message.args)
            if (value && typeof value.then === 'function') {
                this.boundary?.flush()
                const promise = ++this.promise
                this.send({ kind: 'reply', call: message.call, promise })
                void Promise.resolve(value).then(
                    value => this.send({ kind: 'settled', promise, value }),
                    error => this.send({ kind: 'settled', promise, failure: encodeLuaFailure(error) }),
                )
            } else this.send({ kind: 'reply', call: message.call, ...this.strings.encode(value) })
        } catch (error) {
            this.send({ kind: 'reply', call: message.call, failure: encodeLuaFailure(error) })
        }
    }

    isClosed(): boolean { return this.closed }

    private fail(error: Error): void {
        const pending = this.pending
        this.pending = undefined
        this.close()
        pending?.reject(error)
    }

    close(): void {
        if (this.closed) return
        this.closed = true
        this.boundary?.flush()
        this.removeAbort?.()
        this.removeAbort = undefined
        this.transport.release(this.engine)
        this.callbacks.clear()
        this.strings.clear()
        const pending = this.pending
        this.pending = undefined
        pending?.reject(new Error('Lua executor closed'))
    }
}
