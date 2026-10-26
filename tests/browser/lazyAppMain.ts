import { mount } from 'svelte'
import '../../src/styles.css'
import App from '../../src/App.svelte'
import { openRoute, lazyImportBoundary, navigationState, loadedStore, bootFailure, alertStore } from './lazyAppAdapters'
import { appLock } from '../../src/ts/appLock/appLock.svelte'
import { createPinRecord, defaultAppLockRecord } from '../../src/ts/appLock/record'
import { installAppLockInput } from '../../src/ts/appLock/input'
import { createSyncExitCoordinator } from '../../src/ts/storage/syncExitCoordinator'
import { configureSyncExitCoordinator } from '../../src/ts/storage/syncExitProduction'
import { loadV3Plugins } from '../../src/ts/plugins/apiV3/v3.svelte'
import { fixtureState } from './hostAdapters'
import { FileSystemClient } from '../../src/ts/process/mcp/filesystemclient'
import { createHostToolBridge } from '../../src/ts/plugins/hostToolBridge'

installAppLockInput()
const exitCoordinator = createSyncExitCoordinator({
    acquireEditFence: async () => ({ release() {} }),
    flushLocal: async () => { throw new Error('Synthetic local write failure') },
    checkpointLocal: async () => {},
    captureTarget: async () => ({ revision: 1, libraryEpoch: 'synthetic', selectionEpoch: 'synthetic', selectionId: 'synthetic' }),
    selectedDrain: () => null,
})
configureSyncExitCoordinator(exitCoordinator)
const lockDriver = {
    async initialize(nativeUnlock = false) {
        let record = JSON.stringify({ ...defaultAppLockRecord(), enabled: true, timing: 'immediate', nativeUnlock, pin: await createPinRecord('123456') })
        await appLock.initialize({ read: () => record, commit: async value => { record = value } })
    },
    bootFailure(loaded: boolean) {
        loadedStore.set(loaded)
        bootFailure.set({ kind: 'unknown', message: 'Synthetic startup failure', stage: 'synthetic' })
    },
    requestExit() { void exitCoordinator.requestExit() },
    changePin() { appLock.beginChange() },
    loading(value: boolean) { loadedStore.set(!value) },
    loadingAlert() { alertStore.set({ type: 'ask', msg: 'Synthetic startup confirmation' }) },
}
const plugin = {
    async load(script: string) {
        fixtureState.database.plugins = [{ name: 'lock-fixture', script, arguments: {}, realArg: {}, version: '3.0', customLink: [], argMeta: {}, enabled: true }]
        await loadV3Plugins(fixtureState.database.plugins)
    },
    writes: fixtureState.writes,
}
const files = {
    result: null as unknown,
    initialActivation: false,
    async run() {
        this.initialActivation = navigator.userActivation.isActive
        const access = createHostToolBridge({
            loadHostSources: async () => new Map(),
            builtInSourceIds: () => ['internal:fs'],
            createBuiltInSource: async () => new FileSystemClient(),
            pluginSources: () => new Map(),
            pluginSourceOwner: () => undefined,
            captureSelectedConversation: () => ({ characterId: 'synthetic', conversationId: 'synthetic' }),
        }).forPlugin('provider-manager')
        try {
            const { scope } = await access.listTools()
            await access.callTool({ scope, source: 'internal:fs', name: 'fs_write_file', arguments: { path: 'model.txt', content: 'from model' } })
            this.result = await access.callTool({ scope, source: 'internal:fs', name: 'fs_read_file', arguments: { path: 'model.txt' } })
        } catch (error) { this.result = { error: String(error) } }
    },
}
Object.assign(window, { lazyImportBoundary, lazyApp: { open: openRoute, ...navigationState, lock: lockDriver, plugin, files } })
mount(App, { target: document.querySelector<HTMLElement>('#app')! })
if (new URLSearchParams(location.search).has('fsCall')) void files.run()
export type FileSystemDriver = typeof files
