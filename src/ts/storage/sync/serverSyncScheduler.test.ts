import { afterEach, describe, expect, it, vi } from 'vitest'
import { createServerSyncScheduler } from './serverSyncScheduler'

afterEach(() => vi.useRealTimers())
function fixture() {
    vi.useFakeTimers()
    const dependencies = { push: vi.fn(async () => {}), pull: vi.fn(async () => {}), connect: vi.fn(async () => {}), disconnect: vi.fn(async () => {}), failed: vi.fn() }
    return { ...dependencies, scheduler: createServerSyncScheduler(dependencies) }
}
describe('server LWW foreground scheduler', () => {
    it('debounces from the last durable edit and flushes generation completion immediately', async () => {
        const f = fixture(); await f.scheduler.foreground(true); await vi.advanceTimersByTimeAsync(0); f.push.mockClear()
        f.scheduler.localChange(); await vi.advanceTimersByTimeAsync(1500); f.scheduler.localChange()
        await vi.advanceTimersByTimeAsync(1999); expect(f.push).not.toHaveBeenCalled()
        await vi.advanceTimersByTimeAsync(1); expect(f.push).toHaveBeenCalledTimes(1)
        f.scheduler.localChange(true); await vi.advanceTimersByTimeAsync(0); expect(f.push).toHaveBeenCalledTimes(2)
        f.scheduler.dispose()
    })
    it('keeps a stalled receive independent from publication', async () => {
        const f = fixture(); let release!: () => void; f.pull.mockImplementation(() => new Promise<void>(resolve => { release = resolve }))
        await f.scheduler.foreground(true); await vi.advanceTimersByTimeAsync(0); f.push.mockClear()
        f.scheduler.localChange(true); await vi.advanceTimersByTimeAsync(0)
        expect(f.push).toHaveBeenCalledTimes(1); release(); f.scheduler.dispose()
    })
    it('uses foreground fallback and connected safety intervals and stops all work in background', async () => {
        const f = fixture(); await f.scheduler.foreground(true); await vi.advanceTimersByTimeAsync(0); f.pull.mockClear()
        await vi.advanceTimersByTimeAsync(5000); expect(f.pull).toHaveBeenCalledTimes(1)
        f.scheduler.socket(true); f.pull.mockClear(); await vi.advanceTimersByTimeAsync(59_999); expect(f.pull).not.toHaveBeenCalled()
        await vi.advanceTimersByTimeAsync(1); expect(f.pull).toHaveBeenCalledTimes(1)
        await f.scheduler.foreground(false); f.pull.mockClear(); f.push.mockClear(); f.connect.mockClear()
        await vi.advanceTimersByTimeAsync(120_000); f.scheduler.localChange(true); f.scheduler.remoteHint()
        expect(f.pull).not.toHaveBeenCalled(); expect(f.push).not.toHaveBeenCalled(); expect(f.connect).not.toHaveBeenCalled(); expect(f.disconnect).toHaveBeenCalledTimes(1)
    })
    it('immediately pulls on notification, conversation open, foreground return and socket loss', async () => {
        const f = fixture(); await f.scheduler.foreground(true); await vi.advanceTimersByTimeAsync(0); f.pull.mockClear()
        f.scheduler.remoteHint(); await vi.advanceTimersByTimeAsync(0); f.scheduler.conversationOpened(); await vi.advanceTimersByTimeAsync(0); f.scheduler.socket(false); await vi.advanceTimersByTimeAsync(0)
        expect(f.pull).toHaveBeenCalledTimes(3)
        await f.scheduler.foreground(false); await f.scheduler.foreground(true); await vi.advanceTimersByTimeAsync(0); expect(f.pull).toHaveBeenCalledTimes(4)
        f.scheduler.dispose()
    })
    it('retries transient publication with bounded backoff and stops permanent failures', async () => {
        const f = fixture()
        f.push.mockRejectedValueOnce({ code: 'server-unreachable', retryable: true })
        await f.scheduler.foreground(true); await vi.advanceTimersByTimeAsync(0)
        expect(f.push).toHaveBeenCalledTimes(1)
        await vi.advanceTimersByTimeAsync(999); expect(f.push).toHaveBeenCalledTimes(1)
        await vi.advanceTimersByTimeAsync(1); expect(f.push).toHaveBeenCalledTimes(2)
        f.push.mockRejectedValueOnce({ code: 'equal-stamp-integrity', retryable: false })
        f.scheduler.localChange(true); await vi.advanceTimersByTimeAsync(0)
        expect(f.scheduler.isBlocked()).toBe(true)
        f.push.mockClear(); f.pull.mockClear(); f.connect.mockClear()
        await vi.advanceTimersByTimeAsync(120_000)
        expect(f.push).not.toHaveBeenCalled(); expect(f.pull).not.toHaveBeenCalled(); expect(f.connect).not.toHaveBeenCalled()
        f.scheduler.dispose()
    })
    it('serializes explicit receive-to-head in background and fences every active lane', async () => {
        const f = fixture(); const release: Array<() => void> = []
        f.pull.mockImplementation(() => new Promise<void>(resolve => release.push(resolve)))
        const available = f.scheduler.receiveAvailableChanges()
        await vi.advanceTimersByTimeAsync(0)
        expect(f.pull).toHaveBeenCalledWith(true)
        let fenced = false; const fence = f.scheduler.fence().then(() => { fenced = true })
        await vi.advanceTimersByTimeAsync(0); expect(fenced).toBe(false)
        release[0](); await available; await fence; expect(fenced).toBe(true)
        await vi.advanceTimersByTimeAsync(120_000); expect(f.pull).toHaveBeenCalledTimes(1)
        f.scheduler.dispose()
    })
    it('stops native integrity errors restored as Error messages', async () => {
        const f=fixture(); f.pull.mockRejectedValueOnce(new Error('equal-stamp-integrity'))
        await f.scheduler.foreground(true); await vi.advanceTimersByTimeAsync(0)
        expect(f.scheduler.isBlocked()).toBe(true); f.pull.mockClear(); f.push.mockClear()
        await vi.advanceTimersByTimeAsync(120_000); expect(f.pull).not.toHaveBeenCalled(); expect(f.push).not.toHaveBeenCalled()
        f.scheduler.dispose()
    })
})
