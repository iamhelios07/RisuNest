import { Channel, invoke } from '@tauri-apps/api/core'
import type { ExternalJobSummary, ExternalSnapshotExportProgress } from './types'
import { externalJobIsActive, externalJobProgress } from './connection'

export type ExternalStage = 'checking' | 'preparing' | 'uploading' | 'downloading' | 'applying' | 'finalizing'
export interface ExternalTransferSnapshot {
    sequence: number
    stage: ExternalStage
    preparedBytes: string
    uploadedBytes: string
    downloadedBytes: string
    uploadedObjects: string
    downloadedObjects: string
}
export interface ExternalOperationProgress {
    id: string
    connectionId: string
    kind: 'sync' | 'binding' | 'download' | 'export'
    authority?: string
    stage: ExternalStage
    state: 'running' | 'complete' | 'failed' | 'cancelled'
    visible: boolean
    amounts: Omit<ExternalTransferSnapshot, 'sequence' | 'stage'>
    completedItems?: number
    totalItems?: number
    exported?: ExternalSnapshotExportProgress
    previous?: ExternalOperationProgress
}
const emptyAmounts = () => ({ preparedBytes: '0', uploadedBytes: '0', downloadedBytes: '0', uploadedObjects: '0', downloadedObjects: '0' })
const operations = new Map<string, ExternalOperationProgress>()
const listeners = new Set<(value: ReadonlyMap<string, ExternalOperationProgress>) => void>()
const key = (id: string, kind: ExternalOperationProgress['kind']) => `${id}:${kind}`
function emit() { for (const listener of listeners) listener(new Map(operations)) }
export function subscribeExternalProgress(listener: (value: ReadonlyMap<string, ExternalOperationProgress>) => void) {
    listeners.add(listener)
    listener(new Map(operations))
    return () => { listeners.delete(listener) }
}
export function externalProgressFor(value: ReadonlyMap<string, ExternalOperationProgress>, id: string, kind: 'sync' | 'download' | 'export' = 'sync') {
    const binding = kind === 'sync' ? value.get(key(id, 'binding')) : undefined
    const current = value.get(key(id, kind))
    const progress = binding?.state === 'running' ? binding : current?.visible || current?.previous ? current : binding
    return progress?.visible ? progress : progress?.previous
}
export function clearExternalProgress(connectionId: string) {
    for (const kind of ['sync', 'binding', 'download', 'export'] as const) operations.delete(key(connectionId, kind))
    emit()
}
export function clearExternalSyncProgress(connectionId: string) {
    operations.delete(key(connectionId, 'sync'))
    emit()
}

/** One operation owns its channels until it settles; screens only subscribe. */
export function beginExternalProgress(connectionId: string, kind: ExternalOperationProgress['kind']) {
    const slot = key(connectionId, kind)
    const prior = operations.get(slot)
    const previous = kind === 'sync' && prior?.state === 'complete' && prior.visible ? { ...prior, previous: undefined } : prior?.previous
    let value: ExternalOperationProgress = { id: crypto.randomUUID(), connectionId, kind, stage: 'checking', state: 'running', visible: kind !== 'sync', amounts: emptyAmounts(), previous }
    let ended = false
    let channelGeneration = 0
    const current = () => !ended && operations.get(slot)?.id === value.id
    const update = (patch: Partial<ExternalOperationProgress>) => {
        if (!current()) return
        value = { ...value, previous: operations.get(slot)?.previous, ...patch }
        operations.set(slot, value)
        emit()
    }
    operations.set(slot, value)
    emit()
    const reveal = setTimeout(() => update({ visible: true }), 600)
    return {
        stage(stage: ExternalStage, authority?: string) { update({ stage, ...(authority ? { authority } : {}) }) },
        items(completedItems: number, totalItems: number) { update({ stage: 'downloading', completedItems, totalItems }) },
        exported(exported: ExternalSnapshotExportProgress) { update({ stage: 'downloading', exported }) },
        async invoke<T>(command: string, args: Record<string, unknown>, authority?: string): Promise<T> {
            update({ ...(authority ? { authority } : {}) })
            const generation = ++channelGeneration
            const base = value.amounts
            let sequence = -1
            const progress = new Channel<ExternalTransferSnapshot>()
            progress.onmessage = reading => {
                if (!current() || generation !== channelGeneration || reading.sequence <= sequence) return
                if (!['checking', 'preparing', 'uploading', 'downloading', 'applying', 'finalizing'].includes(reading.stage)) return
                const amounts = emptyAmounts()
                for (const field of Object.keys(amounts) as (keyof typeof amounts)[]) {
                    if (!/^\d+$/.test(reading[field])) return
                    amounts[field] = (BigInt(base[field]) + BigInt(reading[field])).toString()
                }
                sequence = reading.sequence
                update({ stage: reading.stage, amounts })
            }
            try { return await invoke<T>(command, { ...args, progress }) }
            finally { if (generation === channelGeneration) channelGeneration++ }
        },
        finish(state: Exclude<ExternalOperationProgress['state'], 'running'>, meaningful = true) {
            clearTimeout(reveal)
            if (!current()) return
            update({ state, visible: state !== 'complete' || meaningful || kind !== 'sync' })
            ended = true
            // One result per scope survives view remounts; successful routine rounds fade away.
            if (state === 'complete') setTimeout(() => {
                const latest = operations.get(slot)
                if (latest?.id === value.id) { operations.delete(slot); emit() }
                else if (latest?.previous?.id === value.id) { operations.set(slot, { ...latest, previous: undefined }); emit() }
            }, 4000)
        },
    }
}
export type ExternalProgressRun = ReturnType<typeof beginExternalProgress>
export const externalProgressFailure = (error: unknown) => {
    const e = error as { name?: string; code?: string; kind?: string } | null
    return e?.name === 'AbortError' || e?.code === 'cancelled' || e?.kind === 'cancelled' ? 'cancelled' : 'failed'
}
export const externalJobTotalGrows = (job: ExternalJobSummary) => job.kind === 'backup' && job.counters === 'transferred' && externalJobIsActive(job)
export function externalJobFraction(job: ExternalJobSummary): number | null {
    return (job.kind === 'restore' && ['preparing-local', 'applying-local', 'awaiting-adoption'].includes(job.phase)) || externalJobTotalGrows(job) ? null : externalJobProgress(job)
}
