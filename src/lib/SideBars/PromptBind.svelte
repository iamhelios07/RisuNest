<script lang="ts">
    import { modalNavigation } from 'src/ts/ui/modalNavigation'
    import { ScrollTextIcon, SearchIcon, XIcon } from '@lucide/svelte'
    import { DBState, selectedCharID, openPresetList } from 'src/ts/stores.svelte'
    import { language } from 'src/lang'
    import { bindPromptPreset, captureChatBindingTarget, chatBindingBlockedByGeneration, saveChatBinding } from 'src/ts/chatBindings.svelte'
    import { activateChatPresetBinding } from 'src/ts/storage/database.svelte'
    import { alertError } from 'src/ts/alert'

    let target = $state<ReturnType<typeof captureChatBindingTarget>>(null)
    let query = $state('')
    let saving = $state(false)
    let chat = $derived(DBState.db.characters[$selectedCharID]?.chats[DBState.db.characters[$selectedCharID]?.chatPage])
    let bound = $derived(DBState.db.botPresets.find(preset => preset.id && preset.id === chat?.bindedBotPreset))
    let globalPreset = $derived(DBState.db.botPresets[DBState.db.botPresetsId])
    let label = $derived(bound?.name ?? (chat?.bindedBotPreset
        ? language.missingBoundPromptPreset
        : `${language.inheritPromptPreset} (${globalPreset?.name ?? language.presets})`))
    let visible = $derived(DBState.db.botPresets.filter(preset =>
        (preset.name ?? '').toLocaleLowerCase().includes(query.trim().toLocaleLowerCase())))

    async function select(id: string | null) {
        const captured = target
        if (saving || !captured?.isCurrent() || chatBindingBlockedByGeneration()) return
        saving = true
        try {
            await bindPromptPreset(captured.conversation, id)
            await saveChatBinding()
            if (captured.isCurrent()) await activateChatPresetBinding()
            target = null
        } catch (error) {
            alertError(language.promptBindingSaveFailed)
        } finally {
            saving = false
        }
    }
</script>

<div class="flex flex-col gap-1 w-full">
    <div class="text-xs text-textcolor2 px-0.5">{language.promptBinding}</div>
    <button
        class="flex items-center gap-2 w-full min-h-10 px-3 py-2 rounded-md bg-darkbutton border text-left transition-colors hover:bg-selected {bound
            ? 'border-selected text-textcolor' : 'border-darkborderc text-textcolor2 hover:text-textcolor'}"
        title={language.promptBinding}
        onclick={() => {
            if (chatBindingBlockedByGeneration()) return
            query = ''
            target = captureChatBindingTarget()
            if (!target) openPresetList.set(true)
        }}
    >
        <ScrollTextIcon size={16} class="shrink-0" />
        <span class="min-w-0 truncate text-sm">{label}</span>
    </button>
</div>
{#if target}
    <div class="fixed inset-0 z-modal bg-black/50 flex items-center justify-center"
        use:modalNavigation={{ close: () => { if (!saving) target = null } }}>
        <div class="bg-darkbg p-4 rounded-md flex flex-col w-124 max-w-full max-h-full overflow-y-auto" role="dialog" aria-modal="true" aria-label={language.promptBinding}>
            <div class="flex items-center justify-between gap-2 mb-3 text-textcolor">
                <h2 class="text-lg">{language.promptBinding}</h2>
                <button disabled={saving} aria-label={language.close} onclick={() => { target = null }}><XIcon size={24} /></button>
            </div>
            <div class="text-sm text-textcolor2 mb-3">{language.promptBindingDesc}</div>
            <div class="flex items-center gap-2 rounded-md border border-darkborderc px-3 mb-2">
                <SearchIcon size={16} class="text-textcolor2 shrink-0" />
                <input class="w-full py-2 text-sm bg-transparent text-textcolor outline-none" placeholder={language.search} aria-label={language.search} bind:value={query} />
            </div>
            <button class="text-left text-textcolor p-3 border-b border-darkborderc hover:bg-selected/30"
                class:bg-selected={!target.conversation.bindedBotPreset} disabled={saving} onclick={() => select(null)}>
                {language.inheritPromptPreset} ({globalPreset?.name ?? language.presets})
            </button>
            {#each visible as preset (preset.id)}
                <button class="flex items-center gap-2 text-left text-textcolor p-3 border-b border-darkborderc hover:bg-selected/30"
                    class:bg-selected={preset.id === target.conversation.bindedBotPreset} disabled={saving || !preset.id} onclick={() => select(preset.id!)}>
                    {#if preset.image}<img src={preset.image} alt="" class="h-7 w-7 shrink-0 rounded-md object-cover" />{/if}
                    <span class="truncate">{preset.name}</span>
                </button>
            {/each}
        </div>
    </div>
{/if}
