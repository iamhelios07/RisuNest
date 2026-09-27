// @vitest-environment happy-dom

import { afterEach, describe, expect, it, vi } from 'vitest'
import { mount, tick, unmount } from 'svelte'
import { get } from 'svelte/store'

vi.mock('src/ts/platform', () => ({ isMobile: false }))
vi.mock('src/ts/stores.svelte', async () => {
    const { writable } = await import('svelte/store')
    return { alertStore: writable({ type: 'none', msg: '' }) }
})

import TextEditorPopup from './TextEditorPopup.svelte'
import { modalNavigation } from 'src/ts/ui/modalNavigation'
import { alertStore } from 'src/ts/stores.svelte'
import {
    openTextEditorPopup,
    textEditorPopup,
    type TextEditorPopupRequest,
} from 'src/ts/gui/textEditorPopup.svelte'

let component: ReturnType<typeof mount> | undefined

afterEach(async () => {
    if (component) await unmount(component)
    component = undefined
    textEditorPopup.request = null
    alertStore.set({ type: 'none', msg: '' })
    document.body.replaceChildren()
})

async function open(overrides: Partial<TextEditorPopupRequest> = {}) {
    const request: TextEditorPopupRequest = {
        value: 'Original text',
        save: vi.fn(() => true),
        cancel: vi.fn(),
        ...overrides,
    }
    openTextEditorPopup(request)
    const target = document.createElement('div')
    document.body.append(target)
    component = mount(TextEditorPopup, { target, props: { request } })
    await tick()
    const textarea = target.querySelector('textarea')!
    const buttons = [...target.querySelectorAll('button')]
    return {
        request,
        target,
        textarea,
        close: buttons[0],
        cancel: buttons.find((button) => button.textContent?.trim() === 'Cancel')!,
        save: buttons.find((button) => button.textContent?.trim() === 'Save')!,
    }
}

function type(textarea: HTMLTextAreaElement, value: string) {
    textarea.value = value
    textarea.dispatchEvent(new Event('input', { bubbles: true }))
}

describe('text editor popup', () => {
    it('shows the text in a titled dialog and focuses it for typing', async () => {
        const { target, textarea } = await open({ title: 'Description' })
        expect(target.querySelector('[role="dialog"]')?.getAttribute('aria-modal')).toBe('true')
        expect(target.querySelector('h2')?.textContent).toBe('Description')
        expect(textarea.value).toBe('Original text')
        await vi.waitFor(() => expect(document.activeElement).toBe(textarea))
    })

    it('saves the edited text and closes', async () => {
        const { request, textarea, save } = await open()
        type(textarea, 'Edited text')
        save.click()
        await vi.waitFor(() => expect(textEditorPopup.request).toBeNull())
        expect(request.save).toHaveBeenCalledWith('Edited text')
        expect(request.cancel).not.toHaveBeenCalled()
    })

    it('reports the draft as it is typed', async () => {
        const input = vi.fn()
        const { textarea } = await open({ input })
        type(textarea, 'Typed draft')
        expect(input).toHaveBeenLastCalledWith('Typed draft')
    })

    it('stays open with the draft when the save is refused', async () => {
        const { request, textarea, save } = await open({ save: vi.fn(async () => false) })
        type(textarea, 'Refused text')
        save.click()
        await vi.waitFor(() => expect(request.save).toHaveBeenCalledWith('Refused text'))
        await tick()
        expect(textEditorPopup.request).toBe(request)
        expect(textarea.value).toBe('Refused text')
        expect(save.disabled).toBe(false)
    })

    it.each(['close', 'cancel'] as const)('discards the draft from the %s button', async (button) => {
        const opened = await open()
        type(opened.textarea, 'Discarded text')
        opened[button].click()
        expect(textEditorPopup.request).toBeNull()
        expect(opened.request.cancel).toHaveBeenCalledOnce()
        expect(opened.request.save).not.toHaveBeenCalled()
    })

    it('saves on Ctrl+Enter and keeps typing keys from the app hotkeys', async () => {
        const hotkeys = vi.fn()
        document.addEventListener('keydown', hotkeys)
        try {
            const { request, textarea } = await open()
            type(textarea, 'Shortcut text')
            const typing = new KeyboardEvent('keydown', { key: '[', ctrlKey: true, bubbles: true, cancelable: true })
            textarea.dispatchEvent(typing)
            textarea.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', ctrlKey: true, bubbles: true, cancelable: true }))
            await vi.waitFor(() => expect(request.save).toHaveBeenCalledWith('Shortcut text'))
            expect(hotkeys).not.toHaveBeenCalled()
        } finally {
            document.removeEventListener('keydown', hotkeys)
        }
    })

    it('discards the draft on Escape and leaves the modal underneath open', async () => {
        const host = document.createElement('div')
        document.body.append(host)
        const closeHost = vi.fn()
        // Settings and other RisuNest modals close on Escape through the same helper.
        const hostNavigation = modalNavigation(host, { close: closeHost })
        try {
            const { request, textarea } = await open()
            type(textarea, 'Discarded text')
            const escape = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true })
            textarea.dispatchEvent(escape)
            expect(escape.defaultPrevented).toBe(true)
            expect(request.cancel).toHaveBeenCalledOnce()
            expect(request.save).not.toHaveBeenCalled()
            expect(textEditorPopup.request).toBeNull()
            expect(closeHost).not.toHaveBeenCalled()
        } finally {
            hostNavigation.destroy()
        }
    })

    it('closes an alert shown over it on Escape before closing itself', async () => {
        const { request, textarea } = await open()
        alertStore.set({ type: 'normal', msg: 'Synthetic notice' })
        textarea.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
        expect(get(alertStore).type).toBe('none')
        expect(request.cancel).not.toHaveBeenCalled()
        expect(textEditorPopup.request).toBe(request)

        textarea.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
        expect(request.cancel).toHaveBeenCalledOnce()
        expect(textEditorPopup.request).toBeNull()
    })

    it('leaves IME Enter and Escape to the input method', async () => {
        const { request, textarea } = await open()
        for (const key of ['Enter', 'Escape']) {
            const event = new KeyboardEvent('keydown', { key, ctrlKey: true, isComposing: true, bubbles: true, cancelable: true })
            textarea.dispatchEvent(event)
            expect(event.defaultPrevented).toBe(false)
        }
        expect(request.save).not.toHaveBeenCalled()
        expect(request.cancel).not.toHaveBeenCalled()
    })

    it('cancels the previous request when another editor opens', () => {
        const first: TextEditorPopupRequest = { value: 'a', save: vi.fn(() => true), cancel: vi.fn() }
        const second: TextEditorPopupRequest = { value: 'b', save: vi.fn(() => true), cancel: vi.fn() }
        openTextEditorPopup(first)
        openTextEditorPopup(second)
        expect(first.cancel).toHaveBeenCalledOnce()
        expect(second.cancel).not.toHaveBeenCalled()
        expect(textEditorPopup.request).toBe(second)
    })
})
