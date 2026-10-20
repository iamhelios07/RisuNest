import { describe, expect, it } from 'vitest'
import { createGenerationDiagnostics, type GenerationDiagnosticAdmission } from './generationDiagnostics'

describe('generation diagnostic boundary', () => {
    it('drops early events and correlates later events with the native document and generation', async () => {
        const events: Record<string, unknown>[] = []
        let ready!: (value: number) => void
        const diagnostics = createGenerationDiagnostics({
            enabled: true,
            invoke: (_command, args) => {
                const event = args.event as Record<string, unknown>
                events.push(event)
                if (event.kind === 'document-start') return new Promise<number>((resolve) => { ready = resolve })
                return Promise.resolve(7)
            },
        })
        diagnostics.startDocument()
        diagnostics.startDocument()
        const first = diagnostics.generation()
        first.phase('send-admitted', 'enter')
        expect(events).toEqual([{ kind: 'document-start' }])
        ready(7)
        await Promise.resolve()
        await Promise.resolve()
        first.phase('host-response', 'exit', { resultKind: 'buffered', outcome: 'ok' })
        diagnostics.generation().phase('send-admitted', 'enter')
        diagnostics.lifecycle('memory-warning')
        expect(events.slice(1)).toEqual([
            expect.objectContaining({
                kind: 'phase', documentSequence: 7, generationSequence: 1, eventSequence: 1,
                phase: 'host-response', edge: 'exit', resultKind: 'buffered', outcome: 'ok',
            }),
            expect.objectContaining({ documentSequence: 7, generationSequence: 2, eventSequence: 2, phase: 'send-admitted' }),
            { kind: 'lifecycle', documentSequence: 7, eventSequence: 3, event: 'memory-warning' },
        ])
    })

    it('forwards only the structural event fields', async () => {
        const events: Record<string, unknown>[] = []
        const diagnostics = createGenerationDiagnostics({
            enabled: true,
            invoke: async (_command, args) => { events.push(args.event as Record<string, unknown>); return 3 },
        })
        diagnostics.startDocument()
        await Promise.resolve()
        const trace = diagnostics.generation()
        const admission = {
            intent: 'normal', displayMode: 'normal', profile: 'normal',
            historyLimitEnabled: true, historyLimitMultiplier: 1.5, route: 'history-window',
            prompt: 'PRIVATE_CONTENT_CANARY', documentSequence: 999,
        } satisfies GenerationDiagnosticAdmission & { prompt: string; documentSequence: number }
        trace.admission(admission)
        trace.persistence({
            boundary: 'checkpoint', savePath: 'windowed', pendingEvents: 4,
            dirtyRecords: 4, unacknowledgedMs: 2000, expectedRevision: 5, acknowledgedRevision: 6,
        })
        expect(JSON.stringify(events)).not.toContain('PRIVATE_CONTENT_CANARY')
        expect(events[1]).toMatchObject({ documentSequence: 3, generationSequence: 1, historyLimitMultiplier: 1.5 })
        expect(events[2]).toMatchObject({ expectedRevision: 5, acknowledgedRevision: 6 })
    })

    it('does not invoke native commands on the web or propagate native failures', async () => {
        let calls = 0
        const web = createGenerationDiagnostics({
            enabled: false,
            invoke: () => { calls++; throw new Error('must not be reached') },
        })
        web.startDocument()
        web.generation().phase('settled', 'exit')
        web.lifecycle('background')
        expect(calls).toBe(0)

        for (const synchronous of [false, true]) {
            const failing = createGenerationDiagnostics({
                enabled: true,
                invoke: () => {
                    if (synchronous) throw new Error('unavailable')
                    return Promise.reject(new Error('unavailable'))
                },
            })
            expect(() => failing.startDocument()).not.toThrow()
            expect(() => failing.generation().phase('settled', 'exit')).not.toThrow()
            await Promise.resolve()
            await Promise.resolve()
        }
        let ready = false
        const laterFailure = createGenerationDiagnostics({
            enabled: true,
            invoke: async () => {
                if (!ready) { ready = true; return 1 }
                throw new Error('diagnostic command failed')
            },
        })
        laterFailure.startDocument()
        await Promise.resolve()
        expect(() => laterFailure.generation().phase('settled', 'exit')).not.toThrow()
        await Promise.resolve()
    })
})
