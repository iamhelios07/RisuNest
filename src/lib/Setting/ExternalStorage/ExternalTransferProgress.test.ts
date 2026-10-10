// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest'
import { mount, tick, unmount } from 'svelte'
const native = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: native.invoke, Channel: class { onmessage = (_value: unknown) => {} } }))
import ExternalTransferProgress from './ExternalTransferProgress.svelte'
import { externalStorageStrings } from './strings'
import { beginExternalProgress, clearExternalProgress } from 'src/ts/storage/sync/external/progress'
import type { ExternalJobSummary } from 'src/ts/storage/sync/external/types'
const strings = externalStorageStrings('ko')
let component: ReturnType<typeof mount> | undefined
let target: HTMLDivElement
const show = async (props: Record<string, unknown> = {}) => {
    target = document.createElement('div'); document.body.append(target)
    component = mount(ExternalTransferProgress, { target, props: { connectionId: 'synthetic', strings, ...props } })
    await tick()
}
afterEach(async () => {
    if (component) await unmount(component)
    component = undefined; target?.remove(); clearExternalProgress('synthetic'); vi.useRealTimers()
})
describe('external storage shared panel', () => {
    it('observes an automatic operation started after mounting and keeps details collapsed', async () => {
        vi.useFakeTimers(); await show()
        const run = beginExternalProgress('synthetic', 'sync')
        native.invoke.mockImplementation(async (_name, { progress }) => progress.onmessage({ sequence: 1, stage: 'uploading', uploadedBytes: '4096', uploadedObjects: '2', downloadedBytes: '0', downloadedObjects: '0', preparedBytes: '2000' }))
        await run.invoke('synthetic', {})
        await vi.advanceTimersByTimeAsync(650); await tick()
        expect(target.textContent).toContain(strings.transfer.uploading)
        expect(target.querySelector('details')?.open).toBe(false)
        expect(target.textContent).toContain('2개 항목')
        expect(target.querySelector('[role="progressbar"]')?.hasAttribute('aria-valuenow')).toBe(false)
        run.finish('complete'); await tick()
        expect(target.textContent).toContain(strings.transfer.syncComplete)
    })
    it('shows onboarding details directly and remains active while applying a completed download', async () => {
        const run = beginExternalProgress('synthetic', 'binding')
        run.stage('applying')
        await show({ onboarding: true, syncing: true })
        expect(target.textContent).toContain(strings.transfer.applying)
        expect(target.querySelector('details')).toBeNull()
        expect(target.querySelector('[role="progressbar"]')?.hasAttribute('aria-valuenow')).toBe(false)
        run.finish('failed'); await tick()
        expect(target.textContent).toContain(strings.failed)
        expect(target.textContent).not.toContain(strings.transfer.connectionComplete)
    })
    it('names sync data and file downloads separately without repeating the title', async () => {
        const binding = beginExternalProgress('synthetic', 'binding')
        binding.stage('downloading')
        const download = beginExternalProgress('synthetic', 'download')
        download.items(1, 3)
        await show()
        expect(target.querySelector('[data-external-progress="binding"]')?.textContent).toContain('동기화 데이터 다운로드 중')
        expect(target.querySelector('[data-external-progress="binding"]')?.textContent).not.toContain('파일 다운로드 중')
        expect(target.querySelector('[data-external-progress="download"]')?.textContent?.match(/파일 다운로드 중/g)).toHaveLength(1)
        binding.finish('complete'); download.finish('complete')
    })
    it('keeps each disclosure independent when transfers overlap', async () => {
        native.invoke.mockImplementation(async (_name, { progress }) => progress.onmessage({ sequence: 1, stage: 'uploading', uploadedBytes: '4', uploadedObjects: '1', downloadedBytes: '0', downloadedObjects: '0', preparedBytes: '0' }))
        const binding = beginExternalProgress('synthetic', 'binding')
        await binding.invoke('synthetic', {})
        const download = beginExternalProgress('synthetic', 'download')
        download.items(1, 3)
        await show()
        const panels = target.querySelectorAll('details')
        expect(panels).toHaveLength(2)
        panels[0].open = true
        panels[0].dispatchEvent(new Event('toggle'))
        await tick()
        expect(panels[0].open).toBe(true)
        expect(panels[1].open).toBe(false)
        binding.finish('complete'); download.finish('complete')
    })
    it('stops activity while a connection waits for confirmation and resumes afterwards', async () => {
        const binding = beginExternalProgress('synthetic', 'binding')
        binding.stage('waiting')
        await show({ onboarding: true, syncing: true })
        expect(target.textContent).toContain('사용자 응답 대기 중')
        expect(target.querySelector('[class*="animate-spin"], [class*="animate-pulse"]')).toBeNull()
        binding.stage('applying'); await tick()
        expect(target.textContent).toContain(strings.transfer.applying)
        expect(target.querySelector('[class*="animate-spin"]')).not.toBeNull()
        binding.finish('complete')
    })
    it('displays preparation and confirmed upload separately, then removes the byte fraction during local restore', async () => {
        const base: ExternalJobSummary = { id: 'backup', connectionId: 'synthetic', kind: 'backup', state: 'running', phase: 'prepare', counters: 'prepared', completedBytes: '20', totalBytes: '100', completedItems: '1', totalItems: '5', startedAtMs: '0', updatedAtMs: '0', transfer: { sequence: 1, stage: 'uploading', preparedBytes: '0', downloadedBytes: '0', downloadedObjects: '0', uploadedBytes: '50', uploadedObjects: '2' } }
        await show({ job: base, onboarding: true })
        expect(target.textContent).toContain(strings.jobCounters.prepared)
        expect(target.textContent).toContain(strings.transfer.uploaded)
        expect(target.querySelector('[role="progressbar"]')?.getAttribute('aria-valuenow')).toBe('20')
        await unmount(component!); target.remove(); component = undefined
        await show({ job: { ...base, kind: 'restore', phase: 'applying-local', counters: 'transferred', completedBytes: '100' }, onboarding: true })
        expect(target.textContent).toContain(strings.restorePhases['applying-local'])
        expect(target.querySelector('[role="progressbar"]')?.hasAttribute('aria-valuenow')).toBe(false)
    })
})
