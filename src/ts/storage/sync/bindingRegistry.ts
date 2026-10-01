import type { BindingTarget, SyncBindingOptions, SyncBindingTransport } from './bindingFlow'
import { createSyncBindingFlow } from './bindingFlow'

type Flow = ReturnType<typeof createSyncBindingFlow>
let bindingFlow: Flow | undefined
const transports = new Map<string, SyncBindingTransport>()
const key = (target: Exclude<BindingTarget, { kind: 'none' }>) => `${target.kind}:${target.connectionId}`

export function registerSyncBindingFlow(flow: Flow): () => void {
    if (bindingFlow && bindingFlow !== flow) throw new Error('Sync binding flow is already registered')
    bindingFlow = flow
    return () => { if (bindingFlow === flow) bindingFlow = undefined }
}

export function registerSyncBindingTransport(target: Exclude<BindingTarget, { kind: 'none' }>, transport: SyncBindingTransport): () => void {
    const id = key(target)
    if (transports.has(id)) throw new Error('Sync binding transport is already registered')
    transports.set(id, transport)
    return () => { if (transports.get(id) === transport) transports.delete(id) }
}

export async function bindSyncTarget(target: Exclude<BindingTarget, { kind: 'none' }>, options: SyncBindingOptions = {}) {
    const transport = transports.get(key(target))
    if (!bindingFlow || !transport) throw new Error('Sync binding transport is unavailable')
    return bindingFlow.bind(target, transport, options)
}

export function bindSyncTargetAsNewDevice(target: Exclude<BindingTarget, { kind: 'none' }>) {
    return bindSyncTarget(target, { mode: 'new-device' })
}

export async function unbindSyncTarget() {
    if (!bindingFlow) throw new Error('Sync binding flow is unavailable')
    return bindingFlow.unbind()
}
