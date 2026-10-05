import { describe, expect, it, vi } from 'vitest'
import { languageKorean } from 'src/lang/ko'
import { createRateMeter, laneDeltas, serverSyncProgressView, type ServerSyncAttempt, type ServerSyncLane } from './serverSyncProgress'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const text = languageKorean.risuNest.serverSync
const lane = (name: ServerSyncLane['lane'], counts: Partial<ServerSyncLane> = {}): ServerSyncLane => ({
    lane: name, active: false, step: 'idle', listed: 0, itemsDone: 0, itemsTotal: 0, filesDone: 0, filesTotal: 0, bytesDone: 0, bytesTotal: 0, sentBytes: 0, receivedBytes: 0, ...counts,
})
const attempt = (fields: Partial<ServerSyncAttempt>): ServerSyncAttempt => ({ startedAt: 0, stages: ['publishing'], active: ['publishing'], current: 'publishing', ...fields })

describe('server sync lane deltas', () => {
    it('subtracts the baseline lane by lane and keeps the live step', () => {
        const baseline = [lane('send', { sentBytes: 100, filesDone: 2 }), lane('receive', { receivedBytes: 50 })]
        const current = [lane('send', { active: true, step: 'uploading', sentBytes: 400, filesDone: 5, filesTotal: 9 }), lane('receive', { receivedBytes: 50 })]
        expect(laneDeltas(current, baseline)).toEqual([
            lane('send', { active: true, step: 'uploading', sentBytes: 300, filesDone: 3, filesTotal: 9 }),
            lane('receive'),
        ])
    })
})

describe('server sync rate meter', () => {
    it('reports no rate from one sample and the recent slope afterwards', () => {
        const meter = createRateMeter(3000)
        meter.add(0, 0)
        expect(meter.rate()).toBeUndefined()
        for (let at = 500; at <= 5000; at += 500) meter.add(at, at * 2)
        expect(meter.rate()).toBe(2000)
        // A stall longer than the window reads as no transfer.
        for (let at = 5500; at <= 8500; at += 500) meter.add(at, 10_000)
        expect(meter.rate()).toBe(0)
        meter.reset()
        expect(meter.rate()).toBeUndefined()
    })
})

describe('server sync progress view', () => {
    it('names the native step and shows uploaded bytes against the planned total', () => {
        const view = serverSyncProgressView(attempt({ lanes: [lane('send', { active: true, step: 'uploading', filesDone: 1, filesTotal: 4, bytesDone: 1024 * 1024, bytesTotal: 4 * 1024 * 1024, sentBytes: 1024 * 1024 })], rate: 512 * 1024 }), text, 65_000)
        expect(view.label).toBe(text.activity.uploading)
        expect(view.detail).toBe('1 / 4 · 1.0 MiB / 4.0 MiB')
        expect(view.fraction).toBe(0.25)
        expect(view.counters.map(counter => [counter.key, counter.value])).toEqual([
            ['bytes', '↑ 1.0 MiB · ↓ 0 bytes'],
            ['rate', '512.0 KiB/s'],
            ['files', '1 / 4'],
            ['elapsed', '01:05'],
        ])
    })
    it('uses the file count when downloads report no byte total', () => {
        const view = serverSyncProgressView(attempt({ stages: ['downloading'], active: ['downloading'], current: 'downloading', lanes: [lane('receive', { active: true, step: 'downloading', filesDone: 3, filesTotal: 12, bytesDone: 3000 })] }), text, 0)
        expect(view.label).toBe(text.activity.downloading)
        expect(view.fraction).toBe(0.25)
    })
    it('reports no share while listing, and counts what was read', () => {
        const view = serverSyncProgressView(attempt({ stages: ['downloading'], active: ['downloading'], current: 'downloading', lanes: [lane('receive', { active: true, step: 'listing', listed: 300 })] }), text, 0)
        expect(view.label).toBe(text.activity.enumerating)
        expect(view.detail).toBe(text.itemsCount.replace('{0}', '300'))
        expect(view.fraction).toBeNull()
    })
    it('falls back to the stage label for work without a native step', () => {
        const view = serverSyncProgressView(attempt({ stages: ['downloading', 'applying'], active: ['applying'], current: 'applying', lanes: [lane('receive')] }), text, 0)
        expect(view.label).toBe(text.progress.applying)
        expect(view.detail).toBe('')
        expect(view.fraction).toBeNull()
    })
    it('marks every running stage active and the rest done, in the order they started', () => {
        const view = serverSyncProgressView(attempt({ stages: ['downloading', 'publishing', 'applying'], active: ['publishing'], current: 'applying' }), text, 0)
        expect(view.stages.map(stage => [stage.stage, stage.state])).toEqual([['downloading', 'done'], ['publishing', 'active'], ['applying', 'done']])
        expect(view.label).toBe(text.progress.publishing)
    })
    it('shows only the elapsed time before the first sample', () => {
        const view = serverSyncProgressView(attempt({}), text, 2_000)
        expect(view.counters).toEqual([{ key: 'elapsed', label: text.elapsed, value: '00:02' }])
    })
    it('lists items only when the step knows their total', () => {
        const counted = serverSyncProgressView(attempt({ lanes: [lane('send', { active: true, step: 'confirming', itemsDone: 0, itemsTotal: 5 })] }), text, 0)
        expect(counted.label).toBe(text.activity.confirming)
        expect(counted.counters.find(counter => counter.key === 'items')?.value).toBe('0 / 5')
        const uncounted = serverSyncProgressView(attempt({ lanes: [lane('send', { active: true, step: 'preparing' })] }), text, 0)
        expect(uncounted.counters.some(counter => counter.key === 'items')).toBe(false)
    })
})
