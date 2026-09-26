import { afterEach, expect, it, vi } from 'vitest'
import { mount, tick, unmount } from 'svelte'
import { languageEnglish } from 'src/lang/en'
import { DBState } from 'src/ts/stores.svelte'
import ModuleSettings from './ModuleSettings.svelte'

vi.mock('src/lang', () => ({ language: languageEnglish }))
vi.mock('src/ts/stores.svelte', () => {
    const DBState = $state({ db: { modules: [], enabledModules: [] } })
    return { DBState }
})
vi.mock('src/ts/process/modules', () => ({}))
vi.mock('src/ts/gui/tooltip', () => ({ tooltip: () => {} }))
vi.mock('src/ts/alert', () => ({}))
vi.mock('src/ts/process/mcp/mcp', () => ({}))
vi.mock('src/ts/interchangeability', () => ({}))
vi.mock('src/ts/characters', () => ({}))
vi.mock('./ModuleMenu.svelte', () => ({ default: () => {} }))

let instance: ReturnType<typeof mount>
afterEach(async () => {
    if (instance) await unmount(instance)
    document.body.replaceChildren()
})

it('creates one module and keeps the same record when creation finishes', async () => {
    instance = mount(ModuleSettings, { target: document.body })
    document.querySelector<HTMLButtonElement>('button')!.click()
    await tick()
    expect(DBState.db.modules).toHaveLength(1)
    const module = DBState.db.modules[0]
    module.name = 'Synthetic module'
    const finish = [...document.querySelectorAll('button')].find(button => button.textContent === languageEnglish.createModule)!
    finish.click()
    await tick()
    expect(DBState.db.modules).toHaveLength(1)
    expect(DBState.db.modules[0]).toBe(module)
    expect(document.body.textContent?.match(/Synthetic module/g)).toHaveLength(1)
})
