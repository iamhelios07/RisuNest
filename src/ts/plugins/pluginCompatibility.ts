import { Mutex } from '../mutex'

interface PluginLoadDependencies<T> {
    resetRegistry(isCurrent: () => boolean): Promise<unknown>
    loadV3(plugins: readonly T[], isCurrent: () => boolean): Promise<unknown>
}

export interface PluginLoadReentrancyGuard {
    runEvaluation<T>(evaluate: () => Promise<T>): Promise<T>
    settle(operation: Promise<void>): Promise<void>
}

export function createPluginLoadReentrancyGuard(
    onBackgroundError: (error: unknown) => void,
): PluginLoadReentrancyGuard {
    let evaluationDepth = 0
    return {
        async runEvaluation<T>(evaluate: () => Promise<T>): Promise<T> {
            evaluationDepth++
            try {
                return await evaluate()
            } finally {
                evaluationDepth--
            }
        },
        settle(operation: Promise<void>): Promise<void> {
            if (evaluationDepth === 0) return operation
            void operation.catch(onBackgroundError)
            return Promise.resolve()
        },
    }
}

export async function runPluginUnloadCallbacks(
    callbacks: Set<() => void | Promise<void>>,
    isCurrent: () => boolean,
    reentrancy: PluginLoadReentrancyGuard,
): Promise<boolean> {
    for (const callback of [...callbacks]) {
        callbacks.delete(callback)
        await reentrancy.runEvaluation(async () => callback())
        if (!isCurrent()) return false
    }
    return isCurrent()
}

export function createPluginLoadOrchestrator<T>(dependencies: PluginLoadDependencies<T>) {
    let loadGeneration = 0
    let latestLoad: Promise<void> = Promise.resolve()
    const operationMutex = new Mutex()

    return (plugins: readonly T[], admit: () => boolean = () => true, allowed: () => boolean = () => true): Promise<void> => {
        const generation = ++loadGeneration
        const isCurrent = () => generation === loadGeneration && allowed()

        const operation = operationMutex.runExclusive(async () => {
            if (!isCurrent() || !admit()) return
            await dependencies.resetRegistry(isCurrent)
            if (!isCurrent()) return

            await dependencies.loadV3(plugins, isCurrent)
        })
        // A caller whose load was replaced waits for the replacement, so the runtime it
        // asked to change has changed when its promise settles.
        const settled = operation.then(() => generation === loadGeneration ? undefined : latestLoad)
        latestLoad = settled
        return settled
    }
}

export function createDeferredPluginReload(dependencies: {
    isIdle(): boolean
    subscribe(changed: () => void): void
    reload(isCurrent: () => boolean): Promise<void>
    onError(error: unknown): void
}) {
    let requested = false
    let running = false
    let scheduled = false
    let epoch = 0
    const schedule = () => {
        if (scheduled) return
        scheduled = true
        queueMicrotask(() => {
            scheduled = false
            if (!requested || running || !dependencies.isIdle()) return
            requested = false
            running = true
            const started = epoch
            void dependencies.reload(() => started === epoch).catch(dependencies.onError).finally(() => {
                running = false
                if (requested) schedule()
            })
        })
    }
    dependencies.subscribe(schedule)
    return {
        request() { requested = true; schedule() },
        cancel() { requested = false; epoch++ },
    }
}
