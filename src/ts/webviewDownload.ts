export type WebviewDownloadHandler = (name: string, source: Blob | string, signal: AbortSignal) => Promise<boolean>

interface WebviewDownloadDependencies {
    fetch: (url: string, options: { signal: AbortSignal }) => Promise<Response>
    save: (name: string, bytes: Uint8Array) => Promise<boolean>
    reportError: (error: unknown) => void | Promise<void>
}

// All WebView downloads share the native picker, which accepts one request at a time.
let pendingDownload: Promise<unknown> = Promise.resolve()

export function createWebviewDownloadHandler(dependencies: WebviewDownloadDependencies): WebviewDownloadHandler {
    return (name, source, signal) => {
        const task = pendingDownload.then(async () => {
            try {
                signal.throwIfAborted()
                if (typeof name !== 'string') throw new Error('Invalid download filename')
                let blob: Blob
                if (source instanceof Blob) {
                    blob = source
                    name ||= source instanceof File ? source.name : 'download'
                } else {
                    if (typeof source !== 'string') throw new Error('Invalid download source')
                    const url = new URL(source)
                    if (!['data:', 'http:', 'https:'].includes(url.protocol)) throw new Error('Unsupported download URL')
                    const response = url.protocol === 'data:'
                        ? await fetch(source, { signal })
                        : await dependencies.fetch(source, { signal })
                    if (!response.ok) throw new Error(`Download failed: HTTP ${response.status}`)
                    blob = await response.blob()
                    if (!name) {
                        try { name = decodeURIComponent(url.pathname.split('/').pop() ?? '') } catch { /* Use a default filename. */ }
                        if (url.protocol === 'data:') name = ''
                        name ||= 'download'
                    }
                }
                const bytes = new Uint8Array(await blob.arrayBuffer())
                signal.throwIfAborted()
                return await dependencies.save(name, bytes)
            } catch (error) {
                if (signal.aborted || (error as { name?: unknown })?.name === 'AbortError') return false
                await dependencies.reportError(error)
                throw error
            }
        })
        pendingDownload = task.catch(() => {})
        return task
    }
}

// Also serialized into opaque plugin frames; keep this function self-contained.
export function installWebviewBridge(
    save: (name: string, source: Blob | string) => Promise<unknown>,
    reportError: (error: unknown) => void,
    openExternal?: (url: string) => void | Promise<unknown>,
    preservePopup?: (url: URL) => boolean,
) {
    const objects = new Map<string, Blob>()
    const create = URL.createObjectURL.bind(URL)
    const revoke = URL.revokeObjectURL.bind(URL)
    URL.createObjectURL = object => {
        const url = create(object)
        if (object instanceof Blob) objects.set(url, object)
        return url
    }
    URL.revokeObjectURL = url => {
        objects.delete(url)
        revoke(url)
    }

    const external = (href: string): URL | undefined => {
        try {
            const url = new URL(href, document.baseURI)
            const base = new URL(document.baseURI)
            if (url.protocol === base.protocol && url.host === base.host
                && (url.host || (url.pathname === base.pathname && url.search === base.search))) return
            if (['http:', 'https:', 'mailto:', 'tel:'].includes(url.protocol)) return url
        } catch { /* Invalid links are not handed to the operating system. */ }
    }
    const open = (url: URL) => {
        try { void Promise.resolve(openExternal!(url.href)).catch(reportError) }
        catch (error) { reportError(error) }
    }
    if (openExternal) {
        const originalOpen = window.open.bind(window)
        window.open = (url, target, features) => {
            if (url) {
                const destination = external(String(url))
                if (destination && preservePopup?.(destination)) return originalOpen(url, target, features)
                if (destination) open(destination)
            }
            return null
        }
    }
    const handle = (event: MouseEvent) => {
        if (event.defaultPrevented || (event.button !== 0 && !(event.type === 'auxclick' && event.button === 1))) return
        const anchor = event.composedPath().find(node => node instanceof HTMLAnchorElement) as HTMLAnchorElement | undefined
        if (!anchor?.hasAttribute('href')) return
        if (!anchor.hasAttribute('download')) {
            if (!openExternal) return
            const target = external(anchor.href)
            if (target) { event.preventDefault(); open(target) }
            return
        }
        const href = anchor.href
        const blob = objects.get(href)
        if (!blob && !/^(?:data:|https?:)/i.test(href)) return
        event.preventDefault()
        // Capture the Blob before the plugin can immediately revoke its URL.
        void save(anchor.download, blob ?? href).catch(reportError)
    }
    window.addEventListener('click', handle)
    if (openExternal) window.addEventListener('auxclick', handle)

    const click = HTMLAnchorElement.prototype.click
    HTMLAnchorElement.prototype.click = function () {
        if (this.isConnected) { click.call(this); return }
        // Detached anchors never bubble to window, but still dispatch their click handlers.
        this.addEventListener('click', handle)
        try { click.call(this) } finally { this.removeEventListener('click', handle) }
    }
    const dispatch = HTMLAnchorElement.prototype.dispatchEvent
    HTMLAnchorElement.prototype.dispatchEvent = function (event) {
        if (event.type === 'click' && event instanceof MouseEvent && !event.cancelable
            && ((this.hasAttribute('download') && /^(?:blob:|data:|https?:)/i.test(this.href))
                || (openExternal && external(this.href)))) {
            // FileSaver dispatches non-cancelable clicks. Suppress the engine's duplicate download.
            event = new MouseEvent('click', {
                bubbles: event.bubbles, cancelable: true, composed: event.composed,
                view: event.view, detail: event.detail,
                screenX: event.screenX, screenY: event.screenY, clientX: event.clientX, clientY: event.clientY,
                ctrlKey: event.ctrlKey, shiftKey: event.shiftKey, altKey: event.altKey, metaKey: event.metaKey,
                button: event.button, buttons: event.buttons, relatedTarget: event.relatedTarget,
            })
        }
        if (event.type !== 'click' || !(event instanceof MouseEvent) || (this.isConnected && event.bubbles)) {
            return dispatch.call(this, event)
        }
        this.addEventListener('click', handle)
        try { return dispatch.call(this, event) } finally { this.removeEventListener('click', handle) }
    }
}
