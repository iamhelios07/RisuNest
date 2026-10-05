import { invoke } from '@tauri-apps/api/core'
import type { languageEnglish } from 'src/lang/en'
import { formatElapsed } from '../../gui/nativeFileJobDialogModel'
import { formatRisuNestStorageBytes } from '../risuNestStorageDashboard'

export type ServerSyncLaneName = 'send' | 'receive' | 'hydrate' | 'binding' | 'assets'
export type ServerSyncNativeStep = 'idle' | 'preparing' | 'checking' | 'uploading' | 'confirming' | 'listing' | 'downloading' | 'staging'
/** One native transport lane. Every count only grows; a view subtracts what it read first. */
export interface ServerSyncLane {
    lane: ServerSyncLaneName
    active: boolean
    step: ServerSyncNativeStep
    listed: number
    itemsDone: number
    itemsTotal: number
    filesDone: number
    filesTotal: number
    bytesDone: number
    /** Planned only where sizes are known before the transfer: uploads and asset downloads. */
    bytesTotal: number
    sentBytes: number
    receivedBytes: number
}
export const readServerSyncLanes = () => invoke<ServerSyncLane[]>('server_sync_progress')

const COUNTS = ['listed', 'itemsDone', 'itemsTotal', 'filesDone', 'filesTotal', 'bytesDone', 'bytesTotal', 'sentBytes', 'receivedBytes'] as const

/** What each lane did after `baseline` was read. */
export function laneDeltas(current: ServerSyncLane[], baseline: ServerSyncLane[]): ServerSyncLane[] {
    return current.map(lane => {
        const base = baseline.find(entry => entry.lane === lane.lane)
        const delta = { ...lane }
        for (const key of COUNTS) delta[key] = Math.max(0, lane[key] - (base?.[key] ?? 0))
        return delta
    })
}

/** Bytes per second over the last few seconds of samples. */
export function createRateMeter(windowMs = 3000) {
    const samples: { at: number; bytes: number }[] = []
    return {
        add(at: number, bytes: number) {
            samples.push({ at, bytes })
            while (samples.length > 2 && at - samples[1].at >= windowMs) samples.shift()
        },
        rate(): number | undefined {
            if (samples.length < 2) return undefined
            const first = samples[0], last = samples[samples.length - 1]
            return last.at > first.at ? Math.max(0, last.bytes - first.bytes) * 1000 / (last.at - first.at) : undefined
        },
        reset() { samples.length = 0 },
    }
}

export type ServerSyncStage = 'preparing' | 'downloading' | 'applying' | 'refreshing' | 'publishing' | 'assets'
export interface ServerSyncAttempt {
    startedAt: number
    /** Stages in the order this attempt first entered them. */
    stages: ServerSyncStage[]
    /** Stages running now, the latest last. */
    active: ServerSyncStage[]
    /** The stage entered last, shown while none is running. */
    current: ServerSyncStage
    /** Lane counts since the attempt was first watched, once a sample arrived. */
    lanes?: ServerSyncLane[]
    rate?: number
}

type Text = (typeof languageEnglish)['risuNest']['serverSync']
export interface ServerSyncProgressView {
    label: string
    detail: string
    /** Share done, or null while the running step reports no total. */
    fraction: number | null
    stages: { stage: ServerSyncStage; label: string; state: 'done' | 'active' }[]
    counters: { key: 'bytes' | 'rate' | 'items' | 'files' | 'elapsed'; label: string; value: string }[]
}

const STAGE_LANES: Record<ServerSyncStage, ServerSyncLaneName[]> = {
    preparing: ['binding'],
    downloading: ['binding', 'receive'],
    applying: [],
    refreshing: [],
    publishing: ['send'],
    assets: ['hydrate', 'assets'],
}
const ACTIVITY: Record<Exclude<ServerSyncNativeStep, 'idle'>, keyof Text['activity']> = {
    preparing: 'preparing',
    checking: 'verifying',
    uploading: 'uploading',
    confirming: 'confirming',
    listing: 'enumerating',
    downloading: 'downloading',
    staging: 'staging',
}
const count = (value: number) => value.toLocaleString()
const ratio = (done: number, total: number) => `${count(Math.min(done, total))} / ${count(total)}`
const bytes = (value: number) => formatRisuNestStorageBytes(value)

/** The lane doing the work of `stage` right now, if a native step runs it. */
function stageLane(attempt: ServerSyncAttempt, stage: ServerSyncStage): ServerSyncLane | undefined {
    return STAGE_LANES[stage]
        .map(name => attempt.lanes?.find(lane => lane.lane === name))
        .find(lane => lane?.active && lane.step !== 'idle')
}
function laneDetail(lane: ServerSyncLane, text: Text): string {
    if (lane.step === 'listing') return lane.listed > 0 ? text.itemsCount.replace('{0}', count(lane.listed)) : ''
    if ((lane.step === 'uploading' || lane.step === 'downloading') && lane.filesTotal > 0) {
        const transferred = lane.bytesTotal > 0 ? `${bytes(lane.bytesDone)} / ${bytes(lane.bytesTotal)}` : bytes(lane.bytesDone)
        return `${ratio(lane.filesDone, lane.filesTotal)} · ${transferred}`
    }
    return lane.itemsTotal > 0 ? ratio(lane.itemsDone, lane.itemsTotal) : ''
}
function laneFraction(lane: ServerSyncLane): number | null {
    if (lane.step !== 'uploading' && lane.step !== 'downloading') return null
    if (lane.bytesTotal > 0) return Math.min(1, lane.bytesDone / lane.bytesTotal)
    return lane.filesTotal > 0 ? Math.min(1, lane.filesDone / lane.filesTotal) : null
}

/** Everything the progress panel shows for a running attempt. */
export function serverSyncProgressView(attempt: ServerSyncAttempt, text: Text, now: number): ServerSyncProgressView {
    const current = attempt.active.at(-1) ?? attempt.current
    const lane = stageLane(attempt, current)
    const stages = attempt.stages.map(stage => ({ stage, label: text.stage[stage], state: attempt.active.includes(stage) ? 'active' as const : 'done' as const }))
    const lanes = attempt.lanes ?? []
    const sum = (key: (typeof COUNTS)[number]) => lanes.reduce((total, entry) => total + entry[key], 0)
    const sent = sum('sentBytes'), received = sum('receivedBytes')
    const itemsDone = sum('itemsDone'), itemsTotal = sum('itemsTotal')
    const filesDone = sum('filesDone'), filesTotal = sum('filesTotal')
    const counters: ServerSyncProgressView['counters'] = []
    if (attempt.lanes) {
        counters.push({ key: 'bytes', label: text.verifiedBytes, value: `↑ ${bytes(sent)} · ↓ ${bytes(received)}` })
        counters.push({ key: 'rate', label: text.transferRate, value: attempt.rate === undefined ? '-' : `${bytes(attempt.rate)}/s` })
        if (itemsTotal > 0) counters.push({ key: 'items', label: text.progressItems, value: ratio(itemsDone, itemsTotal) })
        if (filesTotal > 0) counters.push({ key: 'files', label: text.progressFiles, value: ratio(filesDone, filesTotal) })
    }
    counters.push({ key: 'elapsed', label: text.elapsed, value: formatElapsed(now - attempt.startedAt) })
    return {
        // Asset downloads have one native step, so their stage names them more precisely.
        label: lane && current !== 'assets' ? text.activity[ACTIVITY[lane.step as Exclude<ServerSyncNativeStep, 'idle'>]] : text.progress[current],
        detail: lane ? laneDetail(lane, text) : '',
        fraction: lane ? laneFraction(lane) : null,
        stages,
        counters,
    }
}
