import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

const activity = readFileSync(resolve(process.cwd(), 'src-tauri/gen/android/app/src/main/java/io/github/rsyumi/risunest/MainActivity.kt'), 'utf8')
const bridge = readFileSync(resolve(process.cwd(), 'src-tauri/gen/android/app/src/main/java/io/github/rsyumi/risunest/SafFileBridge.kt'), 'utf8')
const state = readFileSync(resolve(process.cwd(), 'src-tauri/gen/android/app/src/main/java/io/github/rsyumi/risunest/SafDestinationState.kt'), 'utf8')

describe('Android selected document route', () => {
    it('passes selected document ownership to the guarded copy cleanup', () => {
        const start = activity.indexOf('private fun onSafDestinationSelected(')
        const route = activity.slice(start, activity.indexOf('\n  private fun ', start + 1))
        const copy = activity.slice(activity.indexOf('private suspend fun copySafSelectedDestination('), activity.indexOf('private fun cleanupUnclaimedSafDestination('))
        const cleanup = bridge.slice(bridge.indexOf('private fun withPartialCleanup('), bridge.indexOf('private fun checkCancellation('))
        expect(activity).toContain('reservationKind: String? = null')
        expect(activity).toContain('reservedDestination = reservationKind != null')
        expect(route).toContain('copySafSelectedDestination(pending, selectedState)')
        expect(copy).toContain('copySafDestinationOnIo(')
        expect(copy).toContain('createdDocument = selectedState.reservedDestination')
        expect(cleanup).toMatch(/if \(!createdDocument\) \{\s+return SafDestinationException\([\s\S]+?"partial-destination-may-remain"[\s\S]+?\n  \}\s+val removed = runCatching\(deletePartial\)/)
    })

    it('keeps interrupted selected documents and reports possible partial files', () => {
        const start = activity.indexOf('private fun recoverSafDestination(')
        const route = activity.slice(start, activity.indexOf('\n  private fun ', start + 1))
        const cleanup = activity.slice(activity.indexOf('private fun cleanupOwnedSafDestination('), activity.indexOf('private fun cancelUnclaimedReservedDestination('))
        expect(route).toContain('cleanupOwnedSafDestination(record)')
        expect(cleanup).toContain('cleanupReservedSafDestination(record)')
        expect(state).toContain('record.reservedDestination && runCatching { deleteDocument(uri) }')
        expect(state).toContain('else interruptedSafDestinationWarnings()')
    })
})
