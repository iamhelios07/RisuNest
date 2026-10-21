import { DBState } from "../stores.svelte"
import { decompress as fflateDecompress } from "fflate"
import { fetchProtectedResource } from "../sionyw"
import { alertConfirm } from "../alert"
import { language } from "src/lang"
import type { Database } from "../storage/database.svelte"
import { getColdStorageAffectedCharacters, listColdDataKeysFromDb } from "./coldstorageData"
import type { ColdItemRead } from "./coldPayloadExpansion"

export {
    coldStorageHeader,
    getColdStorageBackupKey,
    getColdStorageBackupName,
    isColdStorageBackupData,
    replaceColdStoragePayloadResources,
    listColdDataKeysFromDb
} from "./coldstorageData"

async function decompress(data:Uint8Array) {
    return new Promise<Uint8Array>((resolve, reject) => {
        fflateDecompress(data, (err, decompressed) => {
            if (err) {
                return reject(err)
            }
            resolve(decompressed)
        })
    })
}

async function discardResponseBody(response:Response):Promise<void> {
    try {
        await response.body?.cancel()
    } catch (error) {}
}

/**
 * Reads an upstream account cold item. Import paths expand it into a record
 * body or a plugin value; nothing publishes a cold item back to the account.
 * 200 is present whatever JSON it holds, 404 is missing, and anything else is unreadable.
 */
export async function getAccountColdStorageItem(
    key:string,
    signal?:AbortSignal,
):Promise<ColdItemRead> {
    const d = await fetchProtectedResource('/hub/account/coldstorage', {
        method: 'GET',
        headers: {
            'x-risu-key': key,
        },
        ...(signal ? { signal } : {}),
    })

    if(d.status !== 200){
        await discardResponseBody(d)
        return { kind: d.status === 404 ? 'missing' : 'unreadable' }
    }
    try {
        const buf = await d.arrayBuffer()
        const text = new TextDecoder().decode(await decompress(new Uint8Array(buf)))
        return { kind: 'present', value: JSON.parse(text) }
    } catch (error) {
        if ((error as { name?: string })?.name === 'AbortError') throw error
        console.error(`Failed to decode the account cold storage item ${key}:`, error)
        return { kind: 'unreadable' }
    }
}

export async function listColdDataKeys(db: Pick<Database, 'characters'> = DBState.db): Promise<string[]> {
    return listColdDataKeysFromDb(db)
}

export async function confirmIncompleteColdStorageRestore(
    db: Pick<Database, 'characters'>,
    unavailableKeys: Iterable<string>,
): Promise<boolean> {
    const uniqueUnavailableKeys = Array.from(new Set(unavailableKeys))
    if (uniqueUnavailableKeys.length === 0) {
        return true
    }

    const affected = getColdStorageAffectedCharacters(db, uniqueUnavailableKeys)
    return await alertConfirm(language.errors.coldStorageIncompleteRestoreConfirm(
        affected.characterNames.join(', '),
        uniqueUnavailableKeys.length,
        affected.unresolvedKeys.length,
    ))
}
