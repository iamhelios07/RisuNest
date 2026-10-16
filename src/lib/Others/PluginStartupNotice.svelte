<script module lang="ts">
    import { writable } from 'svelte/store'
    // The plugin settings page shows the notice inline, so the root copy stays hidden while it is open.
    const inlineShown = writable(0)
</script>

<script lang="ts">
    import { language } from 'src/lang'
    import { DBState, settingsOpen, SettingsMenuIndex } from 'src/ts/stores.svelte'
    import { openRisuNestSettingsTab, activeRisuNestSettingsTab } from 'src/ts/setting/risuNestSettingsTabs'
    import { pluginStartupGate, pluginStartupState, retryPluginStartup } from 'src/ts/plugins/pluginStartupGate'
    import { serverSyncErrorMessage } from 'src/ts/storage/sync/serverSyncErrorMessage'
    import { externalErrorMessage, externalStorageStrings } from '../Setting/ExternalStorage/strings'
    import SettingButton from '../Setting/RisuNest/SettingButton.svelte'

    let { inline = false }: { inline?: boolean } = $props()
    let elapsed = $state(false)
    const failed = $derived($pluginStartupState.phase === 'failed')
    const visible = $derived($pluginStartupState.phase !== 'released' && (inline || ($inlineShown === 0 && (elapsed || failed || $pluginStartupState.refused))))
    const error = $derived.by(() => {
        const value = $pluginStartupState.error
        if ($pluginStartupState.target === 'external') return externalErrorMessage(externalStorageStrings(DBState.db.language), value)
        const message = serverSyncErrorMessage(value)
        return message === language.risuNest.serverSync.errorHelp ? '' : message
    })
    $effect(() => {
        const { phase, since } = $pluginStartupState
        elapsed = false
        if (phase === 'released') return
        const timer = setTimeout(() => { elapsed = true }, Math.max(0, since + 1000 - Date.now()))
        return () => clearTimeout(timer)
    })
    $effect(() => {
        if (!inline) return
        inlineShown.update(count => count + 1)
        return () => inlineShown.update(count => count - 1)
    })
    function settings() {
        openRisuNestSettingsTab('sync')
        $SettingsMenuIndex = 17
        $settingsOpen = true
    }
</script>

{#if visible}
    <aside class="rounded-lg border bg-darkbg p-3 text-textcolor {inline ? 'mb-2' : 'shadow-lg'} {failed || $pluginStartupState.refused ? 'border-danger-400' : 'border-darkborderc'}" data-plugin-startup>
        {#if $pluginStartupState.refused}<p role="alert" class="mb-1 font-semibold">{language.risuNest.pluginStartup.requestRefused}</p>{/if}
        {#if failed}
            <div role="alert"><p>{language.risuNest.pluginStartup.syncUnfinished}</p>{#if error}<p class="mt-1 whitespace-pre-wrap text-sm text-textcolor/70">{error}</p>{/if}</div>
            <div class="mt-2 flex flex-wrap items-center gap-2">
                <SettingButton onclick={() => void retryPluginStartup()}>{language.retry}</SettingButton>
                <SettingButton onclick={() => pluginStartupGate.loadOnce()}>{language.risuNest.pluginStartup.loadNow}</SettingButton>
                {#if !($settingsOpen && $SettingsMenuIndex === 17 && $activeRisuNestSettingsTab === 'sync')}
                    <button type="button" class="ml-auto min-h-11 text-sm text-textcolor2 underline" onclick={settings}>{language.risuNest.tabs.sync}</button>
                {/if}
            </div>
        {:else}
            <p role="status" class={$pluginStartupState.refused ? 'text-sm text-textcolor/70' : ''}>{language.risuNest.pluginStartup.waiting}</p>
            <button type="button" class="-my-2 min-h-11 text-sm underline" onclick={() => pluginStartupGate.loadOnce()}>{language.risuNest.pluginStartup.loadNow}</button>
        {/if}
    </aside>
{/if}
