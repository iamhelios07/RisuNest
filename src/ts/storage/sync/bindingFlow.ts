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
    mode?: SyncBindingOptions['mode']
}
export interface NewDeviceBindingPreparation {
    authorizationId: string
    writerId: string
}
export interface NewDeviceBindingResult {
    revision: number
    writerId: string
    bindingAuthority: string
}
export interface SyncBindingOptions {
    mode?: 'new-device'
}
export interface SyncBindingTransport {
    receiveAvailableChanges?(context: BindingContext): Promise<void>
    inspectTarget(context: BindingContext): Promise<InspectedSyncTarget>
    pullAvailableState(target: InspectedSyncTarget, context: BindingContext): Promise<StagedSyncTarget>
    // Native activation only, called under the exclusive gate. It must not acquire a renderer gate.
    replaceFromTarget(staged: StagedSyncTarget, context: BindingContext): Promise<void>
    publishInitialSharedState(context: BindingContext): Promise<void>
    resumeBinding(context: BindingContext): Promise<void>
    fenceOldJobs(context: BindingContext): Promise<void>
    // These hooks reserve/register a writer before activation and use it only after activation.
    prepareNewDeviceBinding?(staged: StagedSyncTarget, context: BindingContext): Promise<NewDeviceBindingPreparation>
    replaceAsNewDevice?(staged: StagedSyncTarget, preparation: NewDeviceBindingPreparation, context: BindingContext): Promise<NewDeviceBindingResult>
    resumeNewDeviceBinding?(preparation: NewDeviceBindingPreparation, result: NewDeviceBindingResult, context: BindingContext): Promise<void>
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
export interface BindingActivationGuard {
    complete(): void
    abortUnchanged(): Promise<void>
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
    beginActivatedLibraryGuard(): BindingActivationGuard
}
export type BindingOutcome = { kind: 'cancelled' } | { kind: 'bound'; action: 'initialized' | 'replaced' | 'resumed' | 'new-device'; state: SyncBindingState; newDevice?: NewDeviceBindingResult }

function sameTarget(a: BindingTarget, b: BindingTarget): boolean {
    return a.kind === b.kind && (a.kind === 'none' || (b.kind !== 'none' && a.connectionId === b.connectionId))
}

function bindingContext(state: SyncBindingState, signal: AbortSignal, mode?: SyncBindingOptions['mode']): BindingContext {
    return { state, signal, ...(mode === 'new-device' ? { mode } : {}) }
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
            await active.transport.fenceOldJobs(bindingContext(active.context.state, active.context.signal, context.mode))
        } else await transport.fenceOldJobs(context)
        await check(context)
    }
    const resumeOld = async (mode?: SyncBindingOptions['mode']) => {
        if (!active) return
        await dependencies.native.assertAuthority(active.context.state)
        const controller = new AbortController()
        active = { ...active, controller, context: bindingContext(active.context.state, controller.signal) }
        await active.transport.resumeBinding(bindingContext(active.context.state, controller.signal, mode))
    }
    return {
        async bind(target: Exclude<BindingTarget, { kind: 'none' }>, transport: SyncBindingTransport, options: SyncBindingOptions = {}): Promise<BindingOutcome> {
            if (running) throw new Error('A sync binding change is already running')
            const mode = options.mode === 'new-device' ? 'new-device' : undefined
            const newDevice = mode === 'new-device'
            if (newDevice && (!transport.prepareNewDeviceBinding || !transport.replaceAsNewDevice || !transport.resumeNewDeviceBinding)) {
                throw new Error('New device sync binding is unavailable')
            }
            running = true
            const controller = new AbortController()
            let originalState: SyncBindingState | undefined
            let jobsFenced = false
            let pluginsFenced = false
            let activationGuard: BindingActivationGuard | undefined
            try {
                const state = await dependencies.native.state()
                originalState = state
                const context = bindingContext(state, controller.signal, mode)
                const inspected = await transport.inspectTarget(context)
                await check(context)
                const replace = newDevice || (!inspected.empty && !inspected.previouslyBoundLibrary)
                let acknowledged = false
                if (newDevice || (replace && await dependencies.hasNonDefaultData())) {
                    if (!await dependencies.confirmReplacement()) return { kind: 'cancelled' }
                    acknowledged = true
                }
                const staged = replace ? await transport.pullAvailableState(inspected, context) : undefined
                await check(context)
                if (staged && (staged.targetId !== inspected.targetId || staged.libraryId !== inspected.libraryId)) {
                    throw new Error('Sync target changed during binding')
                }
                jobsFenced = true
                await fenceOld(transport, context)
                const preparation = newDevice ? await transport.prepareNewDeviceBinding!(staged!, context) : undefined
                await check(context)
                if (replace) {
                    pluginsFenced = true
                    await dependencies.plugins.fenceExecution()
                }
                let newDeviceResult: NewDeviceBindingResult | undefined
                const result: BindingOutcome = await dependencies.withPausedWrites(async () => {
                    await check(context)
                    if (replace && !acknowledged && await dependencies.hasNonDefaultData()) {
                        if (!await dependencies.confirmReplacement()) {
                            pluginsFenced = false
                            await dependencies.plugins.restart()
                            return { kind: 'cancelled' }
                        }
                        acknowledged = true
                    }
                    if (staged) activationGuard = dependencies.beginActivatedLibraryGuard()
                    const next = await dependencies.gate.runTransition(async () => {
                        await check(context)
                        if (preparation) {
                            newDeviceResult = await transport.replaceAsNewDevice!(staged!, preparation, context)
                            const activated = await dependencies.native.state()
                            if (newDeviceResult.writerId !== preparation.writerId || newDeviceResult.bindingAuthority !== activated.targetAuthority ||
                                !sameTarget(activated.target, target) || activated.libraryId !== staged!.libraryId) {
                                throw new Error('New device sync binding changed')
                            }
                            await check(bindingContext(activated, controller.signal, mode))
                            return activated
                        }
                        const next = !replace && sameTarget(state.target, target) && state.libraryId === inspected.libraryId
                            ? state : await dependencies.native.switchTarget(state, target, inspected)
                        const nextContext = bindingContext(next, controller.signal, mode)
                        await check(nextContext)
                        if (staged) await transport.replaceFromTarget(staged, nextContext)
                        await check(nextContext)
                        return next
                    })
                    const nextContext = bindingContext(next, controller.signal, mode)
                    if (staged) {
                        await dependencies.plugins.invalidateCaches()
                        await dependencies.refreshActivatedLibrary()
                        await check(nextContext)
                        activationGuard!.complete()
                        activationGuard = undefined
                        pluginsFenced = false
                        await dependencies.plugins.restart()
                    }
                    await check(nextContext)
                    return { kind: 'bound', action: newDevice ? 'new-device' : staged ? 'replaced' : inspected.empty ? 'initialized' : 'resumed', state: next, ...(newDeviceResult ? { newDevice: newDeviceResult } : {}) }
                })
                if (result.kind === 'cancelled') {
                    jobsFenced = false
                    await resumeOld(mode)
                    return result
                }
                const activatedContext = bindingContext(result.state, controller.signal, mode)
                await check(activatedContext)
                active = { transport, context: bindingContext(result.state, controller.signal), controller }
                jobsFenced = false
                if (result.action === 'new-device') {
                    await transport.resumeNewDeviceBinding!(preparation!, result.newDevice!, activatedContext)
                } else {
                    if (result.action === 'initialized' && await dependencies.hasNonDefaultSharedData()) {
                        await check(activatedContext)
                        await transport.publishInitialSharedState(activatedContext)
                    }
                    await check(activatedContext)
                    await transport.resumeBinding(activatedContext)
                }
                await check(activatedContext)
                return result
            } catch (error) {
                let unchanged = false
                if (originalState && (jobsFenced || pluginsFenced)) {
                    try {
                        const current = await dependencies.native.state()
                        unchanged = current.targetAuthority === originalState.targetAuthority && current.selectionEpoch === originalState.selectionEpoch &&
                            sameTarget(current.target, originalState.target) && current.libraryId === originalState.libraryId
                    } catch { /* Recovery requires a verified unchanged native binding. */ }
                }
                if (unchanged) {
                    try {
                        if (activationGuard) { await activationGuard.abortUnchanged(); activationGuard = undefined }
                        if (pluginsFenced) { pluginsFenced = false; await dependencies.plugins.restart() }
                        if (jobsFenced) { jobsFenced = false; await resumeOld(mode) }
                    } catch (recoveryError) {
                        throw new AggregateError([error, recoveryError], 'Sync binding failed while resuming local state')
                    }
                }
                throw error
            } finally { running = false }
        },
        async unbind(): Promise<SyncBindingState> {
            if (running) throw new Error('A sync binding change is already running')
            running = true
            try {
                const state = await dependencies.native.state()
                if (active) {
                    active.controller.abort()
                    await active.transport.fenceOldJobs(bindingContext(active.context.state, active.context.signal))
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
