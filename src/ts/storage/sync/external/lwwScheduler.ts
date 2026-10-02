export interface LwwSchedulerDependencies {
    available(): boolean
    publish(): Promise<void>
    receive(): Promise<number>
    failed(error: unknown): void
    now?(): number
    setTimer?(callback: () => void, delay: number): unknown
    clearTimer?(timer: unknown): void
}

export function createLwwScheduler(dependencies: LwwSchedulerDependencies) {
    const now = dependencies.now ?? (() => performance.now())
    const setTimer = dependencies.setTimer ?? ((callback, delay) => setTimeout(callback, delay))
    const clearTimer = dependencies.clearTimer ?? (timer => clearTimeout(timer as ReturnType<typeof setTimeout>))
    let enabled = false
    let firstDirty: number | undefined
    let publishTimer: unknown
    let receiveTimer: unknown
    let serial = Promise.resolve()
    let emptyListings = 0
    const queue = <T>(operation: () => Promise<T>): Promise<T> => {
        const task = serial.then(operation)
        serial = task.then(() => {}, error => { dependencies.failed(error) })
        return task
    }
    const clearPublish = () => { if (publishTimer !== undefined) clearTimer(publishTimer); publishTimer = undefined }
    const clearReceive = () => { if (receiveTimer !== undefined) clearTimer(receiveTimer); receiveTimer = undefined }
    const scheduleReceive = () => {
        clearReceive()
        if (!enabled) return
        receiveTimer = setTimer(() => {
            receiveTimer = undefined
            void receiveNow().catch(() => {})
        }, Math.min(120_000, 15_000 * 2 ** Math.min(emptyListings, 3)))
    }
    const receiveNow = async (force = false) => {
        clearReceive()
        if (!force && (!enabled || !dependencies.available())) { scheduleReceive(); return }
        try {
            const count = await queue(dependencies.receive)
            emptyListings = count ? 0 : emptyListings + 1
        } finally { scheduleReceive() }
    }
    const publishNow = async (force = false) => {
        clearPublish()
        if (!force && (!enabled || !dependencies.available())) return
        const captured = firstDirty
        try {
            await queue(dependencies.publish)
            if (firstDirty === captured) firstDirty = undefined
        } catch (error) {
            if (enabled && firstDirty !== undefined) {
                const retryAt = Number((error as { retryAtMs?: string | number })?.retryAtMs)
                publishTimer = setTimer(() => { publishTimer = undefined; void publishNow().catch(() => {}) }, Math.max(15_000, Number.isFinite(retryAt) ? retryAt - Date.now() : 15_000))
            }
            throw error
        }
    }
    return {
        dirty(generationComplete = false) {
            if (firstDirty === undefined) firstDirty = now()
            clearPublish()
            if (!enabled) return
            if (generationComplete) { void publishNow().catch(() => {}); return }
            const delay = Math.max(0, Math.min(15_000, firstDirty + 60_000 - now()))
            publishTimer = setTimer(() => { publishTimer = undefined; void publishNow().catch(() => {}) }, delay)
        },
        start() {
            if (enabled) return
            enabled = true
            emptyListings = 0
            if (firstDirty !== undefined) this.dirty()
            void receiveNow().catch(() => {})
        },
        stop() { enabled = false; clearPublish(); clearReceive() },
        resumeForeground() { emptyListings = 0; if (!enabled) { this.start(); return serial } return receiveNow() },
        conversationOpened() { emptyListings = 0; return receiveNow() },
        publishNow,
        receiveNow,
        settled: () => serial,
    }
}
