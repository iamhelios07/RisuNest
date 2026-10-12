import { invoke } from '@tauri-apps/api/core'
import '../androidNativeControl'
import { language } from 'src/lang'
import { open, SeekMode } from '@tauri-apps/plugin-fs'
import { isTauriAndroid, isTauriIOS } from '../platform'
import { withLibraryFileSelection } from './libraryFileOperation'

export interface NativeSelectedFile {
    path: string
    name: string
    bytes: number
    mimeType: string
}

export interface NativeFileSelectionBridge {
    pick(multiple: boolean): Promise<{ cancelled: boolean; files?: NativeSelectedFile[] }>
    discard(path: string): Promise<void>
}

export interface FileSelectionOptions {
    accept?: string
    multiple?: boolean
    signal?: AbortSignal
}

let selecting = false
const READ_CHUNK_BYTES = 256 * 1024

function bridge(): NativeFileSelectionBridge {
    if (isTauriAndroid) {
        if (!window.RisuNativeFileSelection) throw new Error('Android native file selection is unavailable')
        return window.RisuNativeFileSelection
    }
    if (isTauriIOS) return {
        pick: multiple => invoke('plugin:ios-native|pick_files', { multiple }),
        discard: path => invoke('plugin:ios-native|discard_file', { path }),
    }
    throw new Error('Native mobile file selection is unavailable')
}

function accepts(file: NativeSelectedFile, accept = ''): boolean {
    const types = accept.toLowerCase().split(',').map(type => type.trim()).filter(Boolean)
    return !types.length || types.some(type => type === '*' || type === '*/*'
        || (type.startsWith('.') ? file.name.toLowerCase().endsWith(type)
            : type.endsWith('/*') ? file.mimeType.toLowerCase().startsWith(type.slice(0, -1))
                : file.mimeType.toLowerCase() === type))
}

async function readSelected(file: NativeSelectedFile, signal?: AbortSignal, start = 0, end = file.bytes): Promise<Uint8Array<ArrayBuffer>> {
    signal?.throwIfAborted()
    if (!Number.isSafeInteger(file.bytes) || file.bytes < 0) throw new Error('The selected file has an invalid size')
    const handle = await open(file.path, { read: true })
    let failure: unknown
    try {
        if (start) await handle.seek(start, SeekMode.Start)
        const data = new Uint8Array(end - start)
        let position = 0
        while (position < data.length) {
            signal?.throwIfAborted()
            const count = await handle.read(data.subarray(position, Math.min(position + READ_CHUNK_BYTES, data.length)))
            if (!count) throw new Error('The selected file ended before all bytes were read')
            position += count
        }
        signal?.throwIfAborted()
        if (end === file.bytes && await handle.read(new Uint8Array(1))) throw new Error('The selected file changed while reading')
        return data
    } catch (error) {
        failure = error
        throw error
    } finally {
        try { await handle.close() }
        catch (error) {
            throw new Error([failure, error].filter(Boolean).map(String).join('\n'), { cause: failure ?? error })
        }
    }
}

async function withNativeSelection<T>(options: FileSelectionOptions, consume: (files: NativeSelectedFile[]) => Promise<T>, cancelled: T): Promise<T> {
    return withLibraryFileSelection(async () => {
        if (options.signal?.aborted) return cancelled
        if (selecting) throw new Error('A file picker is already open')
        const native = bridge()
        selecting = true
        let files: NativeSelectedFile[] = []
        let failure: unknown
        try {
            const result = await native.pick(options.multiple ?? false)
            if (result.cancelled) return cancelled
            if (!Array.isArray(result.files) || !result.files.length) throw new Error('The file picker returned no document')
            files = result.files
            if (options.signal?.aborted) return cancelled
            const selected = options.multiple ? files : files.slice(0, 1)
            if (selected.some(file => !accepts(file, options.accept))) {
                throw new Error(language.risuNest.importDialog.reasonUnsupportedFormat)
            }
            return await consume(selected)
        } catch (error) {
            failure = error
            throw error
        } finally {
            const errors: unknown[] = []
            for (const file of files) {
                try { await native.discard(file.path) }
                catch (error) { errors.push(error) }
            }
            selecting = false
            if (errors.length) throw new Error([failure, ...errors].filter(Boolean).map(String).join('\n'), { cause: failure ?? errors[0] })
        }
    })
}

export async function selectNativeFiles(options: FileSelectionOptions): Promise<File[]> {
    return (await selectNativeFileData(options)).map(file => new File([file.data], file.name, { type: file.mimeType }))
}

export function selectNativeFileData(options: FileSelectionOptions = {}) {
    return withNativeSelection(options, async files => {
        const selected: { name: string; data: Uint8Array<ArrayBuffer>; mimeType: string }[] = []
        for (const file of files) {
            selected.push({ name: file.name, data: await readSelected(file, options.signal), mimeType: file.mimeType })
        }
        return selected
    }, [])
}

export interface SelectedFileSlices {
    name: string
    size: number
    slice(start?: number, end?: number): { arrayBuffer(): Promise<ArrayBuffer> }
}

/** The source is owned by this call and remains available until the consumer settles. */
export function withNativeSelectedFile<T>(options: Omit<FileSelectionOptions, 'multiple'>, consume: (file: SelectedFileSlices) => Promise<T>): Promise<T | null> {
    return withNativeSelection<T | null>({ ...options, multiple: false }, async ([file]) => {
        if (!Number.isSafeInteger(file.bytes) || file.bytes < 0) throw new Error('The selected file has an invalid size')
        let active = true
        const offset = (value: number) => {
            const integer = Math.trunc(value) || 0
            return integer < 0 ? Math.max(file.bytes + integer, 0) : Math.min(integer, file.bytes)
        }
        try {
            return await consume({
                name: file.name,
                size: file.bytes,
                slice(start = 0, end = file.bytes) {
                    const from = offset(start)
                    const to = Math.max(from, offset(end))
                    return { async arrayBuffer() {
                        if (!active) throw new Error('The selected file has been released')
                        return (await readSelected(file, options.signal, from, to)).buffer
                    } }
                },
            })
        } finally { active = false }
    }, null)
}
