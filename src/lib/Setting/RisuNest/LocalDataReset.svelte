<script lang="ts">
    import { language } from 'src/lang'
    import { isTauri, isTauriAndroid, isTauriIOS } from 'src/ts/platform'
    import { cleanupNeedsWebViewUpdate, requestAppCleanup } from 'src/ts/storage/appCleanup'
    import SettingGroup from './SettingGroup.svelte'
    import Button from 'src/lib/UI/GUI/Button.svelte'

    let prepareRemoval = $state(false)
    let busy = $state(false)
    let error = $state('')
    let confirming = $state(false)
    const text = $derived(language.risuNest.cleanup)
    async function reset() {
        if (busy) return
        busy = true
        error = ''
        try {
            const requested = await requestAppCleanup({
                native: isTauri,
                desktop: !isTauriAndroid && !isTauriIOS,
                prepareRemoval,
                confirm: async () => confirming,
            })
            if (!requested) busy = false
        } catch (cause) {
            error = cleanupNeedsWebViewUpdate(cause) ? text.webViewUpdateRequired : text.failed
            busy = false
        }
    }
</script>

{#if isTauri}
    <SettingGroup title={text.title} id="risunest-reset" description={text.scope}>
        <div class="space-y-3 p-3">
            <p class="text-sm text-textcolor2">{text.resetHelp}</p>
            {#if !isTauriAndroid && !isTauriIOS}
                <label class="flex items-center gap-2 text-sm">
                    <input type="checkbox" bind:checked={prepareRemoval} disabled={busy || confirming} />
                    {text.prepareRemoval}
                </label>
                <p class="text-sm text-textcolor2">{text.removalHelp}</p>
            {/if}
            {#if confirming}
                <p class="text-sm">{prepareRemoval ? text.confirmRemoval : text.confirmReset}</p>
                <div class="flex flex-wrap gap-2">
                    <Button disabled={busy} onclick={() => void reset()}>{busy ? text.pending : prepareRemoval ? text.deleteAndExit : text.title}</Button>
                    <Button disabled={busy} onclick={() => { confirming = false; error = '' }}>{text.cancel}</Button>
                </div>
            {:else}
                <Button onclick={() => { confirming = true }}>{prepareRemoval ? text.deleteAndExit : text.title}</Button>
            {/if}
            {#if error}<p role="alert" class="text-sm text-textcolor2">{error}</p>{/if}
        </div>
    </SettingGroup>
{/if}
