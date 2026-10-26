// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { flushSync } from 'svelte'
import { toStore } from 'svelte/store'
const mock = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: mock.invoke, Channel: class { onmessage = (_value: unknown) => {} } }))
vi.mock('../mobileBackgroundTask', async () => (await import('../mobileBackgroundTask.testSupport')).mobileBackgroundTaskModule())
vi.mock('src/ts/stores.svelte', () => ({ selectedCharID: { subscribe: (run: (value: number) => void) => { run(0); return () => {} } } }))
vi.mock('./database.svelte', () => ({ getDatabase: () => ({ characters: [] }) }))
import { noteMissingAssetBody, trackAssetBodyArrival } from './assetBodyArrival.svelte'
import { downloadRemoteAssets } from './sync/serverAssetResidency'
import { clearExternalProgress } from './sync/external/progress'
import { assetResidencyStatus } from './sync/serverAssetResidency.testSupport'

beforeEach(() => { mock.invoke.mockReset() })
afterEach(() => { clearExternalProgress('receiver') })

it.each([
    ['finishes', () => Promise.resolve(assetResidencyStatus())],
    ['fails after storing some bodies', () => Promise.reject(new Error('required-asset-unavailable'))],
])('looks up a displayed asset that had no body again when a hydration download %s', async (_case, download) => {
    mock.invoke.mockImplementation(name => name === 'asset_residency_download_remote' ? download() : Promise.resolve())
    // Stands in for a display lookup that found no local body.
    let lookups = 0
    const stop = toStore(() => {
        const token = trackAssetBodyArrival('assets/received-avatar.png')
        if (token === undefined) return lookups
        noteMissingAssetBody('assets/received-avatar.png', token)
        return ++lookups
    }).subscribe(() => {})
    try {
        expect(lookups).toBe(1)
        await downloadRemoteAssets(undefined, { progressConnectionId: 'receiver', showEmpty: false, routine: true }).catch(() => {})
        flushSync()
        expect(lookups).toBe(2)
    } finally { stop() }
})
