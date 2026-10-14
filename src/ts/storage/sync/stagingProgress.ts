/** Current local staging work, independent of cumulative transfer counts. */
export interface StagingProgress {
    phase: 'read' | 'validate' | 'project' | 'copy' | 'check'
    done: number
    total: number | null
}

export function stagingProgressView(progress: StagingProgress | null | undefined, text: Record<StagingProgress['phase'], string> & { items: string }) {
    if (!progress || !Object.hasOwn(text, progress.phase) || !Number.isSafeInteger(progress.done) || progress.done < 0
        || (progress.total !== null && (!Number.isSafeInteger(progress.total) || progress.total < 0))) return undefined
    const known = progress.total !== null && progress.total > 0
    const done = known ? Math.min(progress.done, progress.total!) : progress.done
    return {
        label: text[progress.phase],
        detail: known ? `${done.toLocaleString()} / ${progress.total!.toLocaleString()}` : text.items.replace('{0}', done.toLocaleString()),
        fraction: known ? done / progress.total! : null,
    }
}
