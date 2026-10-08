import { formatElapsed } from '../../gui/nativeFileJobDialogModel'

/** How long a phase must have transferred before its rate predicts the time left. */
export const REMAINING_TIME_WINDOW_MS = 5000

/**
 * The time left of a phase measured in bytes, from that phase's own rate over at least
 * `windowMs`. A phase whose byte total changes after it began has no estimate until another
 * phase begins.
 */
export function createRemainingTimeEstimator(windowMs = REMAINING_TIME_WINDOW_MS) {
    let phase: string | undefined
    let total = 0
    let steady = false
    const samples: { at: number; done: number }[] = []
    const reset = () => { phase = undefined; samples.length = 0 }
    return {
        /** `key` names the running phase, or is undefined while none is measured in bytes. Returns milliseconds left. */
        update(key: string | undefined, done: number, planned: number, at: number): number | undefined {
            if (key === undefined || !(planned > 0)) { reset(); return undefined }
            if (key !== phase) { phase = key; total = planned; steady = true; samples.length = 0 }
            else if (planned !== total) steady = false
            if (!steady) return undefined
            samples.push({ at, done })
            while (samples.length > 2 && at - samples[1].at >= windowMs) samples.shift()
            const first = samples[0]
            if (at - first.at < windowMs || done <= first.done) return undefined
            return Math.max(0, total - done) * (at - first.at) / (done - first.done)
        },
        reset,
    }
}

/** Remaining time in the elapsed-time format, rounded up to whole seconds. */
export const formatRemaining = (milliseconds: number) => formatElapsed(Math.ceil(milliseconds / 1000) * 1000)
