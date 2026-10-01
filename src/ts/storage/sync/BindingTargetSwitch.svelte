<script lang="ts">
    import { bindSyncTarget } from './bindingRegistry'
    import type { BindingOutcome, BindingTarget } from './bindingFlow'

    let { target, label, onBound, onError }: {
        target: Exclude<BindingTarget, { kind: 'none' }>
        label: string
        onBound: (outcome: BindingOutcome) => void
        onError: (error: unknown) => void
    } = $props()
    let busy = $state(false)
    async function connect() {
        if (busy) return
        busy = true
        try { onBound(await bindSyncTarget(target)) }
        catch (error) { onError(error) }
        finally { busy = false }
    }
</script>

<button type="button" class="rounded border border-borderc bg-darkbutton px-3 py-2 text-textcolor disabled:opacity-50" disabled={busy} onclick={connect}>{label}</button>
