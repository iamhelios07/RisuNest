import { describe, expect, it } from 'vitest'
import { createRemainingTimeEstimator, formatRemaining, REMAINING_TIME_WINDOW_MS } from './remainingTime'

describe('remaining time', () => {
    it('estimates nothing until the phase has transferred for the whole window', () => {
        const estimator = createRemainingTimeEstimator()
        const seen = [0, 1000, 2000, 3000, 4000].map(at => estimator.update('phase', at / 20, 1000, at))
        expect(seen).toEqual([undefined, undefined, undefined, undefined, undefined])
        // 50 bytes per second with 750 left.
        expect(estimator.update('phase', 250, 1000, REMAINING_TIME_WINDOW_MS)).toBe(15_000)
        expect(estimator.update('phase', 300, 1000, 6000)).toBe(14_000)
    })
    it('measures the rate over the last window only', () => {
        const estimator = createRemainingTimeEstimator(5000)
        for (let at = 0; at <= 5000; at += 1000) estimator.update('phase', at / 10, 10_000, at)
        // The rate rose from 100 to 1000 bytes per second; the samples older than the window drop out.
        let left: number | undefined
        for (let at = 6000; at <= 12_000; at += 1000) left = estimator.update('phase', 500 + (at - 5000), 10_000, at)
        expect(left).toBe((10_000 - 7500) * 1000 / 1000)
    })
    it('stops estimating a phase whose total changed and starts again with the next phase', () => {
        const estimator = createRemainingTimeEstimator(5000)
        for (let at = 0; at <= 5000; at += 1000) estimator.update('pack', at / 10, 1000, at)
        expect(estimator.update('pack', 600, 1000, 6000)).toBeDefined()
        expect(estimator.update('pack', 700, 2000, 7000)).toBeUndefined()
        expect(estimator.update('pack', 800, 2000, 13_000)).toBeUndefined()
        for (let at = 14_000; at <= 19_000; at += 1000) estimator.update('next', at - 14_000, 10_000, at)
        expect(estimator.update('next', 6000, 10_000, 20_000)).toBe(4000)
    })
    it('has no estimate while nothing moves, for a phase not measured in bytes, or without a total', () => {
        const estimator = createRemainingTimeEstimator(5000)
        for (let at = 0; at <= 6000; at += 1000) expect(estimator.update('stalled', 100, 1000, at)).toBeUndefined()
        expect(estimator.update(undefined, 200, 1000, 7000)).toBeUndefined()
        for (let at = 0; at <= 6000; at += 1000) expect(estimator.update('open', at, 0, at)).toBeUndefined()
    })
    it('rounds the time left up to whole seconds', () => {
        expect(formatRemaining(14_200)).toBe('00:15')
        expect(formatRemaining(400)).toBe('00:01')
        expect(formatRemaining(3_725_000)).toBe('1:02:05')
    })
})
