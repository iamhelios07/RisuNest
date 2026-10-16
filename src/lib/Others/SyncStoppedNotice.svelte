<script lang="ts">
    import { language } from 'src/lang'
    import { externalStorageStrings } from 'src/lib/Setting/ExternalStorage/strings'
    import { pluginStartupState } from 'src/ts/plugins/pluginStartupGate'
    import { activeRisuNestSettingsTab, openRisuNestSettingsTab } from 'src/ts/setting/risuNestSettingsTabs'
    import { syncStatus } from 'src/ts/storage/sync/syncStatus'
    import { syncStatusDisplay } from 'src/ts/storage/sync/syncStatusView'
    import { DBState, settingsOpen, SettingsMenuIndex } from 'src/ts/stores.svelte'
    import SettingButton from '../Setting/RisuNest/SettingButton.svelte'

    // The stop's own label and text, not the local save failure that takes the status button first.
    const display = $derived((DBState.db.risunestSyncStatusUi ?? true) && $syncStatus && syncStatusDisplay($syncStatus, externalStorageStrings(DBState.db.language), false))
    // 닫기 holds until the cause changes or a sync succeeds; retries between two reports of one stop keep it.
    let dismissed = $state<{ key: string; lastSuccessAt?: number }>()
    const visible = $derived(!!display?.stopKey && (dismissed?.key !== display.stopKey || dismissed.lastSuccessAt !== display.lastSuccessAt))

    function settings() {
        openRisuNestSettingsTab('sync')
        $SettingsMenuIndex = 17
        $settingsOpen = true
    }
</script>

<!-- While plugins wait, their notice carries the stop. -->
{#if display && visible && $pluginStartupState.phase === 'released'}
    <section role="alert" class="rounded-lg border border-danger-400 bg-darkbg p-3 text-textcolor shadow-lg" data-sync-stopped>
        <p class="font-semibold">{display.label}</p>
        {#if display.error}<p class="mt-1 whitespace-pre-wrap text-sm text-textcolor/70">{display.error}</p>{/if}
        <div class="mt-2 flex flex-wrap items-center gap-2">
            {#if !($settingsOpen && $SettingsMenuIndex === 17 && $activeRisuNestSettingsTab === 'sync')}
                <SettingButton onclick={settings}>{language.risuNest.tabs.sync}</SettingButton>
            {/if}
            <SettingButton class="ml-auto" onclick={() => { dismissed = { key: display.stopKey, lastSuccessAt: display.lastSuccessAt } }}>{language.close}</SettingButton>
        </div>
    </section>
{/if}
