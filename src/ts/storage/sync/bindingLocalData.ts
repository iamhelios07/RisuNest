import { invoke } from '@tauri-apps/api/core'
import { normalizeDatabaseDefaults, type Database } from '../database.svelte'
import { hasNonDefaultBindingData, type BindingLocalContent } from './bindingDefaults'

interface NativeBindingContent {
    library: Record<string, unknown>
    managedAliasCount: number
    ordinaryPluginValueCount: number
    hypaValueCount: number
    pluginLocalValueCount: number
    opaqueSharedUnitCount?: number
    sharedVariables?: Record<string, unknown>
}

export async function inspectLocalBindingData(): Promise<BindingLocalContent> {
    const native = await invoke<NativeBindingContent>('pds_lww_binding_content')
    const factoryLibrary = structuredClone(normalizeDatabaseDefaults({} as Database)) as unknown as Record<string, unknown>
    return { ...native, factoryLibrary, factoryManagedAliasCount: 0 }
}

export async function hasLocalBindingData(): Promise<boolean> {
    return hasNonDefaultBindingData(await inspectLocalBindingData())
}

export async function hasLocalSharedBindingData(): Promise<boolean> {
    const content = await inspectLocalBindingData()
    return hasNonDefaultBindingData({ ...content, pluginLocalValueCount: 0 })
}
