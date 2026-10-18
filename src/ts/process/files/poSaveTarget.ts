import { writeFile } from '@tauri-apps/plugin-fs'
import { downloadFile } from 'src/ts/globalApi.svelte'
import { isTauriAndroid, isTauriDesktop } from 'src/ts/platform'
import { save } from 'src/ts/storage/fileSelection'
import { publishAndroidSafDownload, reserveAndroidSafDownload } from 'src/ts/storage/androidSafDownload'

export interface PoSaveTarget {
    /** Saves the result so far, where the destination can be rewritten. */
    checkpoint(result: string): Promise<void>
    finish(result: string): Promise<void>
    /** Releases a destination that was never written. */
    close(): Promise<void>
}

/**
 * Asks for the destination once. Desktop rewrites the chosen file after every
 * message and Android writes the destination picked up front at the end; iOS
 * and the web save the final file through the usual download. Resolves null
 * when the picker is cancelled.
 */
export async function openPoSaveTarget(name: string): Promise<PoSaveTarget | null> {
    const encode = (result: string) => new TextEncoder().encode(result)
    if (isTauriDesktop) {
        const path = await save({ defaultPath: name })
        if (!path) return null
        const write = (result: string) => writeFile(path, encode(result))
        return { checkpoint: write, finish: write, close: async () => {} }
    }
    if (isTauriAndroid) {
        const reservation = await reserveAndroidSafDownload(name)
        if (!reservation) return null
        let used = false
        return {
            checkpoint: async () => {},
            async finish(result) {
                used = true
                await publishAndroidSafDownload(reservation, name, encode(result))
            },
            async close() {
                if (used) return
                used = true
                await reservation.release().catch((error) => {
                    console.error('PO save destination release failed', error)
                })
            },
        }
    }
    return {
        checkpoint: async () => {},
        async finish(result) { await downloadFile(name, result) },
        close: async () => {},
    }
}
