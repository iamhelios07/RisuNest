import { createStorageMutationGate } from '../storageMutationGate'
import { createSyncBindingFlow, type BindingPluginLifecycle, type SyncBindingNative } from './bindingFlow'
import { confirmSyncBindingReplacement } from './bindingDialog'
import { hasLocalBindingData, hasLocalSharedBindingData } from './bindingLocalData'
import { createNativeSyncBindingBridge } from './bindingNative'
import { registerSyncBindingFlow } from './bindingRegistry'

export function installSyncBindingFlow(dependencies: {
    native?: SyncBindingNative
    plugins: BindingPluginLifecycle
    withPausedWrites<T>(operation: () => Promise<T>): Promise<T>
    refreshActivatedLibrary(): Promise<void>
}) {
    const flow = createSyncBindingFlow({
        ...dependencies,
        native: dependencies.native ?? createNativeSyncBindingBridge(),
        gate: createStorageMutationGate(),
        hasNonDefaultData: hasLocalBindingData,
        hasNonDefaultSharedData: hasLocalSharedBindingData,
        confirmReplacement: confirmSyncBindingReplacement,
    })
    return { flow, dispose: registerSyncBindingFlow(flow) }
}
