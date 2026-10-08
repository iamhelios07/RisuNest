import { language } from 'src/lang'

/**
 * The alert text for a failure in background persistence work. Native commands reject with
 * plain `{ code, message? }` objects, sometimes as JSON text, so only their message is shown.
 */
export function backgroundErrorMessage(error: unknown): string | Error {
    if (error instanceof Error) return error
    let value = error
    if (typeof value === 'string') {
        try { value = JSON.parse(value) } catch { return value }
        if (!value || typeof value !== 'object') return error as string
    }
    const message = value && typeof value === 'object' ? (value as { message?: unknown }).message : undefined
    if (typeof message === 'string' && message.trim()) return message
    return language.risuNest.backgroundDataFailed
}
