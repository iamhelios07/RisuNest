# Output listener event memory

This harness measures the host JavaScript heap while output listeners run (section 6 of the [listener event design](../../docs/superpowers/specs/2026-10-22-output-listener-event-cost-design.md)). It dispatches one output to two listeners through the real `dispatchChatOutputListeners`, the production projector `createProductionPluginChatOutputProjector`, the pinned position resolver and an IndexedDB store (fake-indexeddb). The synthetic character has ten conversations of 2,600 messages with 2,000 distinct ASCII characters each, 52 MB of message text in total. The live character holds the current conversation and summary stubs for the others, as after navigation. Each variant delivers three outputs.

For each output it records the heap after two forced collections, the highest heap sampled every millisecond and at each listener call, the difference between them, the heap retained after the output, and what each listener received. `full` gives both listeners every conversation. `current` is the device setting that sends only the current conversation. Plugin iframe transfer, plugin-side copies and renderer memory are not measured; heap values are Node values, not device measurements.

Run the current implementation:

```powershell
$env:NODE_OPTIONS = '--expose-gc'
$env:OUTPUT_LISTENER_LABEL = 'after'
pnpm exec vitest run --config benchmarks/output-listener-memory/vitest.config.ts
```

Measure an earlier implementation by replaying preserved sources. The folder mirrors product-relative paths; files it does not contain load from the working tree. The output records the hashes of the loaded listener sources. The implementation before the listener cost reduction only has full history:

```powershell
New-Item -ItemType Directory -Force .tmp/output-listener-memory/before-source/src/ts/plugins
git show 63c91abcb^:src/ts/plugins/pluginChatOutputListeners.ts > .tmp/output-listener-memory/before-source/src/ts/plugins/pluginChatOutputListeners.ts
git show 63c91abcb^:src/ts/plugins/pluginDatabaseAccess.ts > .tmp/output-listener-memory/before-source/src/ts/plugins/pluginDatabaseAccess.ts
$env:OUTPUT_LISTENER_BASELINE_SOURCE = '.tmp/output-listener-memory/before-source'
$env:OUTPUT_LISTENER_VARIANTS = 'full'
$env:OUTPUT_LISTENER_LABEL = 'before'
pnpm exec vitest run --config benchmarks/output-listener-memory/vitest.config.ts
```

Results go to `.tmp/output-listener-memory/<label>.json`, or to `OUTPUT_LISTENER_OUTPUT`. `OUTPUT_LISTENER_SMOKE=1` uses 20 messages per conversation. Without `--expose-gc` the heap values include uncollected garbage, and the output records `gcExposed: false`. `pnpm test:harness` also runs this file against the working tree.
