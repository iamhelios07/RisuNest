import { describe, expect, it, vi } from 'vitest'
import { get } from 'svelte/store'
import { createPluginStartupGate, delayPluginLoadingUntilSync, PLUGIN_SYNC_SETTING } from './pluginStartupGate'

function deferred() {
    let resolve!: () => void
    let reject!: (error: unknown) => void
    const promise = new Promise<void>((yes, no) => { resolve = yes; reject = no })
    return { promise, resolve, reject }
}
function harness() {
    const receive = deferred()
    const h = { enabled: true, delay: true, bound: true, held: false, identity: 'first', starts: 0, reads: 0, received: false }
    const gate = createPluginStartupGate({
        enabled: () => h.enabled, delay: () => h.delay,
        async context() {
            h.reads++
            const identity = h.identity
            return { identity, bound: h.bound, heldByRestore: h.held,
                async receive(signal) { await receive.promise; signal.throwIfAborted(); h.received = true },
                async assertCurrent() { if (identity !== h.identity) throw new Error('changed') },
            }
        },
        async start() { if (await gate.ensure()) h.starts++ },
    })
    return { h, gate, receive }
}

describe('plugin initial receive gate', () => {
    it('returns control to the shell while receive is pending and executes only after completion', async () => {
        const { h, gate, receive } = harness()
        const checking = gate.ensure()
        expect(gate.refuse()).toBe(true)
        expect(await checking).toBe(false)
        expect(get(gate.state).phase).toBe('waiting')
        expect(h.starts).toBe(0)
        expect(gate.refuse()).toBe(true)
        expect(get(gate.state).refused).toBe(true)
        receive.resolve()
        await vi.waitFor(() => expect(h.starts).toBe(1))
        expect(h.received).toBe(true)
        expect(gate.refuse()).toBe(false)
        expect(await gate.ensure()).toBe(true)
    })
    it.each(['bound', 'enabled', 'delay'] as const)('does not wait without %s', async field => {
        const { h, gate } = harness()
        h[field] = false
        expect(await gate.ensure()).toBe(true)
        expect(get(gate.state).phase).toBe('released')
    })
    it('bypasses only a committed server restore hold', async () => {
        const { h, gate } = harness()
        h.held = true
        expect(await gate.ensure()).toBe(true)
        h.held = false
        expect(await gate.ensure()).toBe(false)
    })
    it('keeps a failed receive closed without a timeout, and accepts a one-time load', async () => {
        const { h, gate, receive } = harness()
        await gate.ensure()
        receive.reject(new Error('offline'))
        await vi.waitFor(() => expect(get(gate.state).phase).toBe('failed'))
        expect(h.starts).toBe(0)
        gate.loadOnce()
        await vi.waitFor(() => expect(h.starts).toBe(1))
        expect(h.delay).toBe(true)
        expect(await harness().gate.ensure()).toBe(false)
    })
    it('ignores a completion from the previous binding or cancelled startup attempt', async () => {
        const { h, gate, receive } = harness()
        await gate.ensure()
        gate.invalidate()
        h.identity = 'replacement'
        receive.resolve()
        await Promise.resolve(); await Promise.resolve()
        expect(h.starts).toBe(0)
        expect(get(gate.state).phase).toBe('waiting')
    })
    it('rejects a changed authority even if its completion event has not arrived', async () => {
        const { h, gate, receive } = harness()
        await gate.ensure()
        h.identity = 'replacement'
        receive.resolve()
        await vi.waitFor(() => expect(get(gate.state).phase).toBe('failed'))
        expect(h.starts).toBe(0)
    })
    it('releases only this wait when the preference is unchecked', async () => {
        const { h, gate, receive } = harness()
        await gate.ensure()
        h.delay = false
        gate.preferenceChanged()
        await vi.waitFor(() => expect(h.starts).toBe(1))
        receive.resolve()
        await Promise.resolve(); await Promise.resolve()
        expect(h.starts).toBe(1)
        h.delay = true
        gate.preferenceChanged()
        expect(h.starts).toBe(1)
    })
    it.each([null, 'invalid', '{}', '{"delayPluginLoadingUntilSync":"false"}', '{"delayPluginLoadingUntilSync":true}'])('defaults on for %s', value => {
        localStorage.removeItem(PLUGIN_SYNC_SETTING)
        if (value !== null) localStorage.setItem(PLUGIN_SYNC_SETTING, value)
        expect(delayPluginLoadingUntilSync()).toBe(true)
    })
    it('preserves an explicit false without altering existing preferences', () => {
        localStorage.setItem('risuNestDeviceSettings', 'synthetic-existing-preferences')
        localStorage.setItem(PLUGIN_SYNC_SETTING, '{"delayPluginLoadingUntilSync":false}')
        expect(delayPluginLoadingUntilSync()).toBe(false)
        expect(localStorage.getItem('risuNestDeviceSettings')).toBe('synthetic-existing-preferences')
        localStorage.clear()
    })
})
