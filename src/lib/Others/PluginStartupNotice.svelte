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
    const copy = language.risuNest.pluginStartup
    const failed = $derived($pluginStartupState.phase === 'failed')
    const visible = $derived($pluginStartupState.phase !== 'released' && (inline || elapsed || failed || $pluginStartupState.refused))
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
    function settings() {
        openRisuNestSettingsTab('sync')
        $SettingsMenuIndex = 17
        $settingsOpen = true
    }
</script>

{#if visible}
    <aside class="rounded-lg border bg-darkbg p-3 text-textcolor shadow-lg {failed || $pluginStartupState.refused ? 'border-danger-400' : 'border-darkborderc'}" data-plugin-startup>
        {#if $pluginStartupState.refused}<p role="alert" class="mb-2">{copy.requestRefused}</p>{/if}
        {#if failed}
            <div role="alert"><p>{copy.syncUnfinished}</p>{#if error}<p class="mt-1 whitespace-pre-wrap text-sm">{error}</p>{/if}</div>
        {:else}
            <p role="status">{copy.waiting}</p>
        {/if}
        <div class="mt-2 flex flex-wrap items-center gap-2">
            {#if failed}
                <SettingButton onclick={() => void retryPluginStartup()}>{language.retry}</SettingButton>
                <SettingButton onclick={() => pluginStartupGate.loadOnce()}>{copy.loadNow}</SettingButton>
            {:else}
                <button type="button" class="min-h-11 text-sm underline" onclick={() => pluginStartupGate.loadOnce()}>{copy.loadNow}</button>
            {/if}
            {#if !($settingsOpen && $SettingsMenuIndex === 17 && $activeRisuNestSettingsTab === 'sync')}
                <button type="button" class="min-h-11 text-sm text-textcolor2 underline" onclick={settings}>{language.risuNest.tabs.sync}</button>
            {/if}
        </div>
    </aside>
{/if}
