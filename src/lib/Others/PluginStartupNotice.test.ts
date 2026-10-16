vi.mock('src/ts/parser/parser.svelte', async () => (await import('src/ts/parser/parser.testSupport')).parserStubModule())
import 'src/ts/stores.svelte'
import { afterEach, expect, it, vi } from 'vitest'
import { mount, tick, unmount } from 'svelte'
import { get } from 'svelte/store'
import { changeLanguage, language } from 'src/lang'
import { languageKorean } from 'src/lang/ko'
import { DBState, settingsOpen, SettingsMenuIndex } from 'src/ts/stores.svelte'
import { activeRisuNestSettingsTab } from 'src/ts/setting/risuNestSettingsTabs'
import { configurePluginStartup, pluginStartupState } from 'src/ts/plugins/pluginStartupGate'
import PluginStartupNotice from './PluginStartupNotice.svelte'

let component: ReturnType<typeof mount> | undefined
let inlineComponent: ReturnType<typeof mount> | undefined
afterEach(async () => {
    if (component) await unmount(component)
    if (inlineComponent) await unmount(inlineComponent)
    component = inlineComponent = undefined
    document.body.replaceChildren()
    pluginStartupState.set({ phase: 'released', since: 0 })
    configurePluginStartup(() => false, async () => {})
    settingsOpen.set(false)
    activeRisuNestSettingsTab.set(null)
    DBState.db.risunestSyncStatusUi = true
    vi.useRealTimers()
    changeLanguage('en')
})
async function render() {
    component = mount(PluginStartupNotice, { target: document.body })
    await tick()
}
const labels = () => [...document.querySelectorAll('aside button')].map(button => button.getAttribute('aria-label') ?? button.textContent?.trim())
const close = () => document.querySelector<HTMLButtonElement>(`aside button[aria-label="${language.close}"]`)!.click()

it('waits one second, closes until the next wait or failure, and hides after release', async () => {
    vi.useFakeTimers()
    const since = Date.now()
    pluginStartupState.set({ phase: 'waiting', since })
    await render()
    expect(document.querySelector('aside')).toBeNull()
    await vi.advanceTimersByTimeAsync(999)
    expect(document.querySelector('aside')).toBeNull()
    await vi.advanceTimersByTimeAsync(1); await tick()
    expect(document.querySelector('[role="status"]')?.textContent).toBe(language.risuNest.pluginStartup.waiting)
    close(); await tick()
    expect(document.querySelector('aside')).toBeNull()

    // A failure keeps the wait's start time and still shows.
    pluginStartupState.set({ phase: 'failed', since, error: { code: 'unauthorized' }, target: 'server' }); await tick()
    expect(document.querySelector('[role="alert"]')?.textContent).toContain(language.risuNest.pluginStartup.syncUnfinished)
    close(); await tick()
    expect(document.querySelector('aside')).toBeNull()

    pluginStartupState.set({ phase: 'waiting', since: Date.now() })
    await vi.advanceTimersByTimeAsync(1000); await tick()
    expect(document.querySelector('[role="status"]')).not.toBeNull()
    pluginStartupState.set({ phase: 'released', since: 0 }); await tick()
    expect(document.querySelector('aside')).toBeNull()
})

it('reports only a failure while the sync UI is turned off', async () => {
    vi.useFakeTimers()
    DBState.db.risunestSyncStatusUi = false
    pluginStartupState.set({ phase: 'waiting', since: Date.now() })
    await render()
    await vi.advanceTimersByTimeAsync(1000); await tick()
    expect(document.querySelector('aside')).toBeNull()
    pluginStartupState.set({ phase: 'failed', since: Date.now(), error: { code: 'unauthorized' }, target: 'server' }); await tick()
    expect(document.querySelector('[role="alert"]')?.textContent).toContain(language.risuNest.pluginStartup.syncUnfinished)
})

it('uses the language loaded after the app mounted', async () => {
    vi.useFakeTimers()
    pluginStartupState.set({ phase: 'waiting', since: Date.now() })
    await render()
    changeLanguage('ko')
    await vi.advanceTimersByTimeAsync(1000); await tick()
    expect(document.querySelector('[role="status"]')?.textContent).toBe(languageKorean.risuNest.pluginStartup.waiting)
    expect(labels()).toEqual([languageKorean.close, languageKorean.risuNest.pluginStartup.loadNow])
})

it('shows failure actions immediately and opens the sync tab', async () => {
    pluginStartupState.set({ phase: 'failed', since: Date.now(), error: { code: 'unauthorized' }, target: 'server' })
    await render()
    expect(document.querySelector('[role="alert"]')?.textContent).toContain(language.risuNest.pluginStartup.syncUnfinished)
    expect(labels()).toEqual([language.risuNest.tabs.sync, language.close, language.retry, language.risuNest.pluginStartup.loadNow])
    const settings = [...document.querySelectorAll('button')].find(button => button.textContent === language.risuNest.tabs.sync)!
    settings.click(); await tick()
    expect(get(settingsOpen)).toBe(true)
    expect(get(SettingsMenuIndex)).toBe(17)
    activeRisuNestSettingsTab.set('sync'); await tick()
    expect([...document.querySelectorAll('button')].some(button => button.textContent === language.risuNest.tabs.sync)).toBe(false)
})

it('leaves the plugin settings page to its inline notice', async () => {
    pluginStartupState.set({ phase: 'failed', since: Date.now(), error: { code: 'unauthorized' }, target: 'server' })
    await render()
    inlineComponent = mount(PluginStartupNotice, { target: document.body, props: { inline: true } })
    await tick()
    expect(document.querySelectorAll('aside')).toHaveLength(1)
    await unmount(inlineComponent); inlineComponent = undefined; await tick()
    expect(document.querySelectorAll('aside')).toHaveLength(1)
})
