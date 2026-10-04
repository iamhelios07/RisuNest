// @vitest-environment happy-dom

import { afterEach, describe, expect, it, vi } from 'vitest'
import { mount, tick, unmount } from 'svelte'

vi.mock('src/ts/gui/guisize', async () => {
    const { writable } = await import('svelte/store')
    return { textAreaSize: writable(0), textAreaTextSize: writable(1) }
})
vi.mock('src/ts/gui/highlight', () => ({
    highlighter: vi.fn(), getNewHighlightId: () => 1, removeHighlight: vi.fn(), AllCBS: [],
}))
vi.mock('src/ts/stores.svelte', async () => {
    const { writable } = await import('svelte/store')
    return {
        DBState: { db: { hotkeys: [] } },
        disableHighlight: writable(false),
        popUpEditorStore: { open: false, value: '', mode: '', language: '' },
    }
})
vi.mock('src/ts/platform', () => ({ isMobile: false }))
vi.mock('src/ts/hotkey', () => ({ hotkeyMatches: () => false }))
vi.mock('src/ts/util', () => ({ sleep: async () => {} }))

import TextAreaInput from './TextAreaInput.svelte'
import { textEditorPopup } from 'src/ts/gui/textEditorPopup.svelte'

let component: ReturnType<typeof mount> | undefined

afterEach(async () => {
    if (component) await unmount(component)
    component = undefined
    textEditorPopup.request = null
    document.body.replaceChildren()
})

async function input(props: Record<string, unknown>) {
    const target = document.createElement('div')
    document.body.append(target)
    component = mount(TextAreaInput, { target, props: { value: 'Field text', ...props } })
    await tick()
    const expand = target.querySelector<HTMLButtonElement>('button[aria-label="Edit in popup"]')!
    return { target, expand }
}

describe('text area popup editor button', () => {
    it('opens the popup editor with the field text', async () => {
        const { expand } = await input({})
        expect(expand).not.toBeNull()
        expand.click()
        expect(textEditorPopup.request?.value).toBe('Field text')
    })

    it.each([false, true])('writes the saved text back and reports it to change listeners (highlight %s)', async (highlight) => {
        const onInput = vi.fn()
        const changes: string[] = []
        // Like the toggle text areas, the value is not bound and persists only from the change event.
        const onchange = (event: { currentTarget: HTMLTextAreaElement | HTMLDivElement }) => {
            changes.push(event.currentTarget instanceof HTMLDivElement
                ? event.currentTarget.textContent ?? ''
                : event.currentTarget.value)
        }
        const { target, expand } = await input({ highlight, onInput, onchange })
        expand.click()

        await expect(textEditorPopup.request!.save('Saved text')).resolves.toBe(true)
        expect(onInput).toHaveBeenCalled()
        expect(changes).toEqual(['Saved text'])
        const field = target.querySelector<HTMLElement>(highlight ? '[contenteditable]' : 'textarea')!
        expect(highlight ? field.textContent : (field as HTMLTextAreaElement).value).toBe('Saved text')
    })

    it('closes its popup editor without saving when the field goes away', async () => {
        const onchange = vi.fn()
        const { expand } = await input({ onchange })
        expand.click()
        // The editor renders before anything can remove the field.
        await tick()
        const request = textEditorPopup.request!
        const cancel = vi.spyOn(request, 'cancel')

        await unmount(component!)
        component = undefined
        expect(textEditorPopup.request).toBeNull()
        expect(cancel).toHaveBeenCalledOnce()
        expect(onchange).not.toHaveBeenCalled()
    })

    it('leaves another field popup editor open when it goes away', async () => {
        const { expand } = await input({})
        expand.click()
        await expect(textEditorPopup.request!.save('Saved text')).resolves.toBe(true)
        const other = { value: 'other', save: () => true }
        textEditorPopup.request = other

        await unmount(component!)
        component = undefined
        expect(textEditorPopup.request).toBe(other)
    })
})
