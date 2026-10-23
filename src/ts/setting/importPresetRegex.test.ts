import { setEffectivePresetBinding } from '../storage/effectiveIdentityState'
import { expect, it, vi } from 'vitest'
import { importPresetRegex } from './importPresetRegex'
import type { Database, customscript } from '../storage/database.svelte'
it.each(['finish', 'cancel', 'switch'] as const)('preserves current regex ownership on %s', async mode => {
    const old = [{ comment: 'old' }] as customscript[]
    const fresh = [{ comment: 'fresh' }] as customscript[]
    const incoming = [{ comment: 'imported' }] as customscript[]
    const database = { botPresetsId: 0, botPresets: [{ name: 'A' }, { name: 'B' }], presetRegex: old } as Database
    let finish!: (value: customscript[]) => void
    const failure = vi.fn()
    const task = importPresetRegex(() => database, () => new Promise(resolve => { finish = resolve }), failure)
    database.presetRegex = fresh
    if (mode === 'switch') database.botPresetsId = 1
    finish(mode === 'cancel' ? [] : incoming)
    await task
    expect(database.presetRegex).toEqual(mode === 'finish' ? [...fresh, ...incoming] : fresh)
    expect(old).toEqual([{ comment: 'old' }])
    expect(failure).toHaveBeenCalledTimes(mode === 'switch' ? 1 : 0)
})

it.each(['replacement', 'reorder'] as const)('rejects a same-name %s while the picker is open', async mode => {
    const fresh = [{ comment: 'fresh' }] as customscript[]
    const incoming = [{ comment: 'imported' }] as customscript[]
    const database = { botPresetsId: 0, botPresets: [{ name: 'X', mainPrompt: 'A' }, { name: 'X', mainPrompt: 'B' }], presetRegex: fresh } as Database
    let finish!: (value: customscript[]) => void
    const failure = vi.fn()
    const task = importPresetRegex(() => database, () => new Promise(resolve => { finish = resolve }), failure)
    if (mode === 'replacement') database.botPresets[0] = { name: 'X', mainPrompt: 'C' } as Database['botPresets'][number]
    else database.botPresets.reverse()
    finish(incoming)
    await task
    expect(database.presetRegex).toBe(fresh)
    expect(database.presetRegex).toEqual([{ comment: 'fresh' }])
    expect(failure).toHaveBeenCalledOnce()
})


it('does not import regex into a different chat-bound prompt while the global index is unchanged', async () => {
    const database = { botPresetsId: 0, botPresets: [{ id: 'a', name: 'A' }, { id: 'b', name: 'B' }], presetRegex: [] } as unknown as Database
    let finish!: (value: customscript[]) => void
    const failure = vi.fn()
    const task = importPresetRegex(() => database, () => new Promise(resolve => { finish = resolve }), failure)
    setEffectivePresetBinding(database, 'b', () => {})
    finish([{ comment: 'imported' }] as customscript[])
    await task
    expect(database.presetRegex).toEqual([])
    expect(failure).toHaveBeenCalledOnce()
})
