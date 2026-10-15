vi.mock('src/ts/parser/parser.svelte', async () => (await import('src/ts/parser/parser.testSupport')).parserStubModule())
import 'src/ts/stores.svelte'
import { afterEach, expect, it, vi } from 'vitest'
import { mount, tick, unmount } from 'svelte'
import { get } from 'svelte/store'
import { language } from 'src/lang'
import { settingsOpen, SettingsMenuIndex } from 'src/ts/stores.svelte'
import { activeRisuNestSettingsTab } from 'src/ts/setting/risuNestSettingsTabs'
import { configurePluginStartup, pluginStartupGate, pluginStartupState } from 'src/ts/plugins/pluginStartupGate'
import PluginStartupNotice from './PluginStartupNotice.svelte'

let component: ReturnType<typeof mount> | undefined
afterEach(async () => {
    if (component) await unmount(component)
    component = undefined
    document.body.replaceChildren()
    pluginStartupState.set({ phase: 'released', refused: false, since: 0 })
    configurePluginStartup(() => false, async () => {})
    settingsOpen.set(false)
    activeRisuNestSettingsTab.set(null)
    vi.useRealTimers()
})
async function render() {
    component = mount(PluginStartupNotice, { target: document.body })
    await tick()
}

it('waits one second, announces a refusal immediately, and hides after release', async () => {
    vi.useFakeTimers()
    configurePluginStartup(() => true, async () => {})
    pluginStartupState.set({ phase: 'waiting', refused: false, since: Date.now() })
    await render()
    expect(document.querySelector('aside')).toBeNull()
    await vi.advanceTimersByTimeAsync(999)
    expect(document.querySelector('aside')).toBeNull()
    await vi.advanceTimersByTimeAsync(1); await tick()
    expect(document.querySelector('[role="status"]')?.textContent).toBe(language.risuNest.pluginStartup.waiting)
    pluginStartupGate.refuse(); await tick()
    expect(document.querySelector('[role="alert"]')?.textContent).toBe(language.risuNest.pluginStartup.requestRefused)
    pluginStartupState.set({ phase: 'released', refused: false, since: 0 }); await tick()
    expect(document.querySelector('aside')).toBeNull()
})

it('shows failure actions immediately and opens the sync tab', async () => {
    pluginStartupState.set({ phase: 'failed', refused: false, since: Date.now(), error: { code: 'unauthorized' }, target: 'server' })
    await render()
    expect(document.querySelector('[role="alert"]')?.textContent).toContain(language.risuNest.pluginStartup.syncUnfinished)
    const buttons = [...document.querySelectorAll('button')]
    expect(buttons.map(button => button.textContent?.trim())).toEqual([
        language.retry, language.risuNest.pluginStartup.loadNow, language.risuNest.tabs.sync,
    ])
    buttons.at(-1)!.click(); await tick()
    expect(get(settingsOpen)).toBe(true)
    expect(get(SettingsMenuIndex)).toBe(17)
    activeRisuNestSettingsTab.set('sync'); await tick()
    expect([...document.querySelectorAll('button')].some(button => button.textContent === language.risuNest.tabs.sync)).toBe(false)
})
