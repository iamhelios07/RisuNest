<script lang="ts">
    import { onMount, untrack } from 'svelte'
    import { XIcon } from '@lucide/svelte'
    import { language } from 'src/lang'
    import { isMobile } from 'src/ts/platform'
    import { isCompositionKey } from 'src/ts/hotkeyModifier'
    import { modalNavigation } from 'src/ts/ui/modalNavigation'
    import {
        cancelTextEditorPopup,
        closeTextEditorPopup,
        type TextEditorPopupRequest,
    } from 'src/ts/gui/textEditorPopup.svelte'
    import SettingButton from '../Setting/RisuNest/SettingButton.svelte'

    interface Props {
        request: TextEditorPopupRequest
    }

    let { request }: Props = $props()

    const strings = language.risuNest.textEditor
    let draft = $state(untrack(() => request.value))
    let saving = $state(false)
    let textarea: HTMLTextAreaElement | undefined = $state()
    let viewportTop = $state(0)
    let viewportHeight: number | null = $state(null)

    async function save() {
        if (saving) return
        saving = true
        try {
            if (await request.save(draft)) closeTextEditorPopup(request)
        } finally {
            saving = false
        }
    }

    function cancel() {
        if (saving) return
        cancelTextEditorPopup(request)
    }

    function handleKeydown(event: KeyboardEvent) {
        // App hotkeys listen on the document and would act on the screen behind the editor.
        // Escape and Back are handled by modalNavigation.
        event.stopPropagation()
        if (isCompositionKey(event)) return
        if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) {
            event.preventDefault()
            void save()
        }
    }

    onMount(() => {
        // modalNavigation focuses the close button. On touch devices focus stays there, so the
        // keyboard does not open over the text before the reader picks where to edit.
        const focusTimer = isMobile ? undefined : setTimeout(() => textarea?.focus({ preventScroll: true }))

        // Follow the visual viewport so an on-screen keyboard shrinks the editor instead of
        // covering its bottom and the buttons.
        const viewport = window.visualViewport
        const update = () => {
            viewportTop = viewport.offsetTop
            viewportHeight = viewport.height
        }
        if (viewport) {
            update()
            viewport.addEventListener('resize', update)
            viewport.addEventListener('scroll', update)
        }
        return () => {
            clearTimeout(focusTimer)
            viewport?.removeEventListener('resize', update)
            viewport?.removeEventListener('scroll', update)
        }
    })
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
    class="fixed inset-x-0 z-modal flex bg-black/60 outline-hidden sm:items-center sm:justify-center sm:p-6 lg:p-10"
    style:top="{viewportTop}px"
    style:height={viewportHeight === null ? '100%' : `${viewportHeight}px`}
    tabindex="-1"
    role="presentation"
    data-text-editor-popup
    onkeydown={handleKeydown}
>
    <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="text-editor-popup-title"
        use:modalNavigation={{ close: cancel }}
        class="flex h-full w-full flex-col bg-darkbg text-textcolor sm:max-w-6xl sm:rounded-xl sm:border sm:border-darkborderc sm:shadow-2xl"
    >
        <div class="flex shrink-0 items-center justify-between gap-3 border-b border-darkborderc pr-2 pl-4 pt-[max(0.5rem,env(safe-area-inset-top))] pb-2 sm:pt-2 sm:pl-5">
            <h2 id="text-editor-popup-title" class="min-w-0 truncate text-base font-semibold">{request.title ?? language.edit}</h2>
            <button
                type="button"
                class="shrink-0 rounded-md p-2 text-textcolor2 transition-colors duration-200 hover:bg-selected hover:text-textcolor focus:outline-hidden focus-visible:ring-2 focus-visible:ring-selected"
                aria-label={strings.close}
                title={strings.close}
                onclick={cancel}
            >
                <XIcon size={20} />
            </button>
        </div>
        <textarea
            bind:this={textarea}
            bind:value={draft}
            oninput={(event) => request.input?.(event.currentTarget.value)}
            class="min-h-0 flex-1 resize-none bg-transparent px-4 py-3 text-base leading-relaxed text-textcolor outline-hidden sm:px-5 sm:py-4"
            aria-labelledby="text-editor-popup-title"
            autocomplete="off"
            spellcheck="false"
        ></textarea>
        <div class="flex shrink-0 justify-end gap-2 border-t border-darkborderc px-4 pt-3 pb-[max(0.75rem,env(safe-area-inset-bottom))] sm:px-5 sm:pb-3">
            <SettingButton variant="secondary" class="min-w-20" disabled={saving} onclick={cancel}>{language.cancel}</SettingButton>
            <SettingButton class="min-w-20" busy={saving} onclick={save}>{strings.save}</SettingButton>
        </div>
    </div>
</div>
