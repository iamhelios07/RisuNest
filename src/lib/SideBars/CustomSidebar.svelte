<script lang="ts">
    import { DBState, loadoutModalStore } from 'src/ts/stores.svelte'
    import Button from '../UI/GUI/Button.svelte';
    import { language } from 'src/lang';
    import { getFullSettingsData } from 'src/ts/setting/utils';
    import SettingRenderer from '../Setting/SettingRenderer.svelte';
    import ModelBind from './ModelBind.svelte'
    import PersonaBind from './PersonaBind.svelte'
    import PromptBind from './PromptBind.svelte'
</script>


<div class="rounded-sm flex flex-col w-full gap-2">

    {#each DBState.db.customSidebarItems as item}
        {#if item.type === 'model'}
            <ModelBind />
        {:else if item.type === 'preset'}
            <PromptBind />
        {:else if item.type === 'loadout'}
            <Button onclick={() => {
                loadoutModalStore.open = !loadoutModalStore.open
            }}>{DBState.db.lastLoadedLoadoutName || language.loadouts}</Button>
        {:else if item.type === 'persona'}
            <PersonaBind />
        {:else if item.type === 'setting'}
            {@const setting = getFullSettingsData().find(s => s.id === item.subType)}
            {#if setting}
                <SettingRenderer items={[setting]} />
            {/if}
        {/if}
    {/each}
</div>
