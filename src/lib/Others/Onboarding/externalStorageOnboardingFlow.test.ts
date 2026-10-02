import { describe, expect, it, vi } from 'vitest'
import { mergeExternalHistoryItems } from 'src/ts/storage/sync/external/connection'
import type {
    ExternalConnectionSummary,
    ExternalHistoryItem,
} from 'src/ts/storage/sync/external/types'
import {
    externalOnboardingRestorable,
    externalOnboardingRestoreAreas,
    externalOnboardingRestoreRestarts,
} from './externalStorageOnboardingFlow'

function connection(
    purpose: 'backup' | 'sync',
): Pick<ExternalConnectionSummary, 'purpose'> {
    return { purpose }
}

function item(
    id: string,
    createdAtMs: string,
    overrides: Partial<ExternalHistoryItem> = {},
): ExternalHistoryItem {
    return {
        id,
        snapshotId: id,
        kind: 'recovery-candidate',
        createdAtMs: createdAtMs as ExternalHistoryItem['createdAtMs'],
        logicalRevision: '1' as ExternalHistoryItem['logicalRevision'],
        pinned: false,
        complete: true,
        verified: true,
        includedSections: ['hypa', 'local-plugins', 'local-settings'],
        sameDevice: false,
        ...overrides,
    }
}
describe('external storage onboarding action', () => {
    it('restores the complete backup scope and rejects incomplete entries', () => {
        expect(externalOnboardingRestoreAreas(item('one', '1'))).toEqual([
            'library', 'referencedAssets', 'hypa', 'local-plugins', 'local-settings',
        ])
        expect(() => externalOnboardingRestoreAreas(item('two', '1', {
            includedSections: ['hypa', 'local-settings'],
        }))).toThrow('complete restore scope')
        expect(externalOnboardingRestoreRestarts()).toBe(false)
    })})
