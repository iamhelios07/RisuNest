import { invoke } from '@tauri-apps/api/core'
import { isTauri } from './platform'

export type GenerationDiagnosticPhase =
    | 'send-admitted' | 'history-materialization' | 'prompt-preparation'
    | 'awaiting-provider' | 'host-response' | 'semantic-output' | 'output-application' | 'checkpoint'
    | 'current-chat-parser' | 'output-trigger' | 'inlay' | 'output-listener'
    | 'tts' | 'completion-processing' | 'final-acknowledgement' | 'settled'
export type GenerationDiagnosticOutcome = 'ok' | 'failed' | 'skipped' | 'stale' | 'aborted'
export type GenerationCheckpointSkipReason =
    | 'ineligible' | 'compatibility' | 'missing-capture' | 'complete-gate'
    | 'storage-failed' | 'ownership-lost'
export type GenerationSavePath = 'windowed' | 'complete' | 'fallback' | 'unknown'
export type GenerationDiagnosticFallback =
    | 'none' | 'compatibility' | 'missing-capture' | 'unsupported' | 'unknown'
    | 'selected-conversation-unavailable' | 'standard-hypa-v3-disabled'
    | 'selected-conversation-not-windowed' | 'custom-or-remote-tokenizer'
    | 'experimental-hypa-v3' | 'plugin-editprocess' | 'plugin-editoutput'
    | 'plugin-chat-output' | 'generation-trigger' | 'history-regex'
    | 'old-history-lorebook-consumer' | 'additional-text-consumer'
    | 'inlay-view-consumer' | 'dynamic-history-indirection' | 'explicit-history-reference'

export interface GenerationDiagnosticAdmission {
    intent: 'normal' | 'continue' | 'reroll'
    displayMode: 'off' | 'normal' | 'strong'
    profile: 'normal' | 'low-spec'
    historyLimitEnabled: boolean
    historyLimitMultiplier?: number
    route: 'complete' | 'windowed' | 'history-window' | 'summary-aware' | 'unknown'
    fallback?: GenerationDiagnosticFallback
    admittedMessages?: number
    totalMessages?: number
}

export interface GenerationDiagnosticPhaseDetails {
    outcome?: GenerationDiagnosticOutcome
    skipReason?: GenerationCheckpointSkipReason
    resultKind?: 'buffered' | 'streaming' | 'multiline' | 'failed'
    savePath?: GenerationSavePath
}

export interface GenerationPersistenceDiagnostic {
    boundary: 'checkpoint' | 'final'
    savePath: GenerationSavePath
    pendingEvents?: number
    dirtyRecords?: number
    unacknowledgedMs?: number
    expectedRevision?: number
    acknowledgedRevision?: number
}

export interface GenerationDiagnosticTrace {
    admission(value: GenerationDiagnosticAdmission): void
    phase(phase: GenerationDiagnosticPhase, edge: 'enter' | 'exit', details?: GenerationDiagnosticPhaseDetails): void
    persistence(value: GenerationPersistenceDiagnostic): void
}

type DiagnosticInvoke = (command: string, args: Record<string, unknown>) => Promise<unknown>

export function createGenerationDiagnostics(options: { enabled: boolean; invoke: DiagnosticInvoke }) {
    let documentSequence: number | null = null
    let generationSequence = 0
    let eventSequence = 0
    let started = false
    const send = (event: Record<string, unknown>) => {
        if (!options.enabled) return
        try {
            void options.invoke('native_generation_diagnostic', {
                event: { ...event, eventSequence: ++eventSequence },
            }).catch(() => {})
        } catch {
            // Diagnostics cannot affect the operation being observed.
        }
    }
    return {
        startDocument(): void {
            if (!options.enabled || started) return
            started = true
            try {
                void options.invoke('native_generation_diagnostic', { event: { kind: 'document-start' } })
                    .then((value) => {
                        if (typeof value === 'number' && Number.isSafeInteger(value) && value >= 0) {
                            documentSequence = value
                        }
                    }).catch(() => {})
            } catch {
                // An unavailable native boundary leaves diagnostics disabled.
            }
        },
        generation(): GenerationDiagnosticTrace {
            const sequence = ++generationSequence
            const emit = (event: Record<string, unknown>) => {
                if (documentSequence === null) return
                send({ ...event, documentSequence, generationSequence: sequence })
            }
            return {
                admission: (value) => emit({
                    kind: 'admission', intent: value.intent, displayMode: value.displayMode,
                    profile: value.profile, historyLimitEnabled: value.historyLimitEnabled,
                    historyLimitMultiplier: value.historyLimitMultiplier, route: value.route,
                    fallback: value.fallback, admittedMessages: value.admittedMessages,
                    totalMessages: value.totalMessages,
                }),
                phase: (phase, edge, details) => emit({
                    kind: 'phase', phase, edge, outcome: details?.outcome,
                    skipReason: details?.skipReason, resultKind: details?.resultKind,
                    savePath: details?.savePath,
                }),
                persistence: (value) => emit({
                    kind: 'persistence', boundary: value.boundary, savePath: value.savePath,
                    pendingEvents: value.pendingEvents, dirtyRecords: value.dirtyRecords,
                    unacknowledgedMs: value.unacknowledgedMs,
                    expectedRevision: value.expectedRevision, acknowledgedRevision: value.acknowledgedRevision,
                }),
            }
        },
        lifecycle(event: 'background' | 'stop' | 'exit' | 'expiry' | 'memory-warning' | 'trim-memory', trimLevel?: number): void {
            if (documentSequence === null) return
            send({ kind: 'lifecycle', documentSequence, event, ...(trimLevel === undefined ? {} : { trimLevel }) })
        },
    }
}

export const generationDiagnostics = createGenerationDiagnostics({ enabled: isTauri, invoke })
