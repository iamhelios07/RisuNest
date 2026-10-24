import { afterEach, expect, it, vi } from 'vitest'
import { mount, tick, unmount } from 'svelte'
import { language } from 'src/lang'
import { DBState } from 'src/ts/stores.svelte'
import SeparateParametersSection from './SeparateParametersSection.svelte'

vi.mock('src/ts/stores.svelte', () => {
    const DBState = $state({ db: {} })
    return { DBState }
})
vi.mock('src/ts/model/modellist', () => ({ getModelInfo: () => ({ parameters: ['temperature'], flags: [] }) }))
vi.mock('src/ts/globalApi.svelte', () => ({}))
vi.mock('src/ts/util', () => ({}))
vi.mock('src/lib/Others/Help.svelte', () => ({ default: () => {} }))

let instance: ReturnType<typeof mount> | undefined
afterEach(async () => {
    if (instance) await unmount(instance)
    instance = undefined
    document.body.replaceChildren()
})

const headers = () => [...document.querySelectorAll('button')]
    .filter(button => button.querySelector(':scope > span'))

it('edits only the auxiliary parameter sets and leaves per-model parameters untouched', async () => {
    const overrides = { 'synthetic-model': { temperature: 70 } }
    DBState.db = {
        seperateParametersEnabled: true,
        seperateParameters: { memory: {}, emotion: {}, translate: {}, otherAx: {}, overrides: structuredClone(overrides) },
        seperateModelsForAxModels: false,
        subModel: 'synthetic-model',
    } as any
    instance = mount(SeparateParametersSection, { target: document.body })
    await tick()
    headers()[0].click()
    await tick()
    const sets = headers().slice(1)
    for (const button of sets) {
        button.click()
        await tick()
        // Turns on the set's first parameter, the temperature.
        button.nextElementSibling!.querySelector<HTMLInputElement>('input[type="checkbox"]')!.click()
        await tick()
    }
    for (const set of ['memory', 'emotion', 'translate', 'otherAx']) {
        expect(DBState.db.seperateParameters[set].temperature).toBe(0)
    }
    expect(DBState.db.seperateParameters.overrides).toEqual(overrides)
    expect(sets.map(button => button.textContent.trim())).toEqual([
        language.longTermMemory, language.emotionImage, language.translator, language.others,
    ])
})
