import type { StorageMutationGate } from '../storageMutationGate'

export type BindingTarget = { kind: 'server' | 'external'; connectionId: string } | { kind: 'none' }
export interface SyncBindingState {
    target: BindingTarget
    targetAuthority: string
    selectionEpoch: string
    libraryId: string | null
    progress: unknown
}
export interface InspectedSyncTarget {
    inspectionId: string
    targetId: string
    libraryId: string
    empty: boolean
    previouslyBoundLibrary: boolean
}
export interface StagedSyncTarget {
    targetId: string
    libraryId: string
    stagingId: string
    receiveId: string
}
export interface BindingContext {
    state: SyncBindingState
    signal: AbortSignal
}
export interface SyncBindingTransport {
    inspectTarget(context: BindingContext): Promise<InspectedSyncTarget>
    pullAvailableState(target: InspectedSyncTarget, context: BindingContext): Promise<StagedSyncTarget>
    // Native activation only, called under the exclusive gate. It must not acquire a renderer gate.
    replaceFromTarget(staged: StagedSyncTarget, context: BindingContext): Promise<void>
    publishInitialSharedState(context: BindingContext): Promise<void>
    resumeBinding(context: BindingContext): Promise<void>
    fenceOldJobs(context: BindingContext): Promise<void>
}
export interface SyncBindingNative {
    state(): Promise<SyncBindingState>
    switchTarget(expected: SyncBindingState, target: BindingTarget, inspection: InspectedSyncTarget | null): Promise<SyncBindingState>
    assertAuthority(state: SyncBindingState): Promise<void>
}
export interface BindingPluginLifecycle {
    fenceExecution(): Promise<void>
    invalidateCaches(): Promise<void>
    restart(): Promise<void>
}
export interface SyncBindingDependencies {
    native: SyncBindingNative
    gate: StorageMutationGate
    plugins: BindingPluginLifecycle
    withPausedWrites<T>(operation: () => Promise<T>): Promise<T>
    hasNonDefaultData(): Promise<boolean>
    hasNonDefaultSharedData(): Promise<boolean>
    confirmReplacement(): Promise<boolean>
    refreshActivatedLibrary(): Promise<void>
}
export type BindingOutcome = { kind: 'cancelled' } | { kind: 'bound'; action: 'initialized' | 'replaced' | 'resumed'; state: SyncBindingState }

function sameTarget(a: BindingTarget, b: BindingTarget): boolean {
    return a.kind === b.kind && (a.kind === 'none' || (b.kind !== 'none' && a.connectionId === b.connectionId))
}

export function createSyncBindingFlow(dependencies: SyncBindingDependencies) {
    let running = false
    let active: { transport: SyncBindingTransport; context: BindingContext; controller: AbortController } | undefined
    const check = async (context: BindingContext) => {
        context.signal.throwIfAborted()
        await dependencies.native.assertAuthority(context.state)
        context.signal.throwIfAborted()
    }
    const fenceOld = async (transport: SyncBindingTransport, context: BindingContext) => {
        if (active) {
            active.controller.abort()
            await active.transport.fenceOldJobs(active.context)
        } else await transport.fenceOldJobs(context)
        await check(context)
    }
    const resumeOld = async () => {
        if (!active) return
        const controller = new AbortController()
        active = { ...active, controller, context: { state: active.context.state, signal: controller.signal } }
        await active.transport.resumeBinding(active.context)
    }
    return {
        async bind(target: Exclude<BindingTarget, { kind: 'none' }>, transport: SyncBindingTransport): Promise<BindingOutcome> {
            if (running) throw new Error('A sync binding change is already running')
            running = true
            const controller = new AbortController()
            try {
                const state = await dependencies.native.state()
                const context = { state, signal: controller.signal }
                const inspected = await transport.inspectTarget(context)
                await check(context)
                const replace = !inspected.empty && !inspected.previouslyBoundLibrary
                let acknowledged = false
                if (replace && await dependencies.hasNonDefaultData()) {
                    if (!await dependencies.confirmReplacement()) return { kind: 'cancelled' }
                    acknowledged = true
                }
                const staged = replace ? await transport.pullAvailableState(inspected, context) : undefined
                await check(context)
                if (staged && (staged.targetId !== inspected.targetId || staged.libraryId !== inspected.libraryId)) {
                    throw new Error('Sync target changed during binding')
                }
                await fenceOld(transport, context)
                if (replace) await dependencies.plugins.fenceExecution()
                const result: BindingOutcome = await dependencies.withPausedWrites(async () => {
                    await check(context)
                    if (replace && !acknowledged && await dependencies.hasNonDefaultData()) {
                        if (!await dependencies.confirmReplacement()) {
                            await dependencies.plugins.restart()
                            return { kind: 'cancelled' }
                        }
                        acknowledged = true
                    }
                    const next = await dependencies.gate.runTransition(async () => {
                        await check(context)
                        const next = !replace && sameTarget(state.target, target) && state.libraryId === inspected.libraryId
                            ? state : await dependencies.native.switchTarget(state, target, inspected)
                        const nextContext = { state: next, signal: controller.signal }
                        await check(nextContext)
                        if (staged) await transport.replaceFromTarget(staged, nextContext)
                        await check(nextContext)
                        return next
                    })
                    const nextContext = { state: next, signal: controller.signal }
                    if (staged) {
                        await dependencies.plugins.invalidateCaches()
                        await dependencies.refreshActivatedLibrary()
                        await check(nextContext)
                        await dependencies.plugins.restart()
                    }
                    await check(nextContext)
                    return { kind: 'bound', action: staged ? 'replaced' : inspected.empty ? 'initialized' : 'resumed', state: next }
                })
                if (result.kind === 'cancelled') {
                    await resumeOld()
                    return result
                }
                const activatedContext = { state: result.state, signal: controller.signal }
                active = { transport, context: activatedContext, controller }
                if (result.action === 'initialized') {
                    if (await dependencies.hasNonDefaultSharedData()) await transport.publishInitialSharedState(activatedContext)
                } else if (result.action === 'resumed') await transport.resumeBinding(activatedContext)
                await check(activatedContext)
                return result
            } finally { running = false }
        },
        async unbind(): Promise<SyncBindingState> {
            if (running) throw new Error('A sync binding change is already running')
            running = true
            try {
                const state = await dependencies.native.state()
                if (active) {
                    active.controller.abort()
                    await active.transport.fenceOldJobs(active.context)
                }
                return await dependencies.withPausedWrites(async () => {
                    const next = await dependencies.gate.runTransition(() => dependencies.native.switchTarget(state, { kind: 'none' }, null))
                    active = undefined
                    return next
                })
            } finally { running = false }
        },
    }
}
