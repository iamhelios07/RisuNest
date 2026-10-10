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
        expect(target.textContent).toContain('2개 객체')
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
