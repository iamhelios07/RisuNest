# LWW synthetic measurement harness

This independent native entry generates synthetic inputs and validates measurement evidence. Product code must never import it. Native store drivers belong in `persistent_store/benchmark*` behind `cfg(test)`; transport smoke drivers are completed after Wave 2, and maintenance drivers after Wave 3. The `plan` command reports pending drivers, not performance results.

Run from the checkout using PowerShell:

```powershell
pwsh -NoProfile -File benchmarks/lww-native/run.ps1 -Action test
pwsh -NoProfile -File benchmarks/lww-native/run.ps1 -Action compile
pwsh -NoProfile -File benchmarks/lww-native/run.ps1 -Action plan
pwsh -NoProfile -File benchmarks/lww-native/run.ps1 -Action generate -Scale small -Output .tmp/lww-measurement/small
```

Create the parent output directory first. Every generated fixture requires a new directory and refuses an existing destination. Keep results in `.tmp/lww-measurement/`, separate from product output and all installed app directories. No command reads an installation or backup. The wrapper resolves the main checkout through Git's common directory and shares its `src-tauri/target`; environment overrides last only for the wrapper invocation. On other hosts use Cargo directly with the same shared target policy.

`target` streams at least 1 GiB of database JSON and 100,000 asset descriptors. `above` streams at least 1.25 GiB and 100,001 assets. A JSON `FixtureScale` can request larger sizes or a different deterministic seed, without a fixed asset or database cap. The long conversation stays at 4,096 messages at every default scale, so append and middle-edit inputs are fixed while unrelated characters grow. Database serialization grows in 64-message character batches and reports actual bytes, not an estimated size. A 1 GiB fixture means serialized database bytes, not SQLite file bytes. Final drivers must report the imported SQLite size separately.

`database.json` is a synthetic upstream-shaped library with stable preset, persona, character, conversation and message IDs. `assets.jsonl` is a streamed catalog, and `fixture.json` records actual counts, sizes and SHA-256 identities. Asset bodies are unique deterministic 1 KiB inputs reconstructed by `asset_body(seed,index)`. Add `-Bodies` to materialize every hash-named body for filesystem restore tests. Catalog-only fixtures do not prove body reuse or restore behavior. Drivers stage the synthetic catalog using native APIs and must attach synthetic asset owners when their scenario requires reachability; the library JSON does not embed 100,000 references in one character.

## Callable driver contract

`measurement::run` accepts a `MeasurementDriver`, a scale, a stated network profile, scenarios, warmup count and repetition count. `prepare` receives the identical `FixedChange` at every scale. `run` must reset test-only counters before each operation, perform the actual product operation, wait for the tested phase completions, and return a `Sample`. Missing instrumentation is an error. `CounterEvidence` distinguishes an observed zero from an unavailable counter. Tests of the runner use simulated samples only and are not performance evidence.

`PhaseClock` marks durable save, publication, foreground apply, usable library and all bodies local from one operation start. `summarize` reports nearest-rank p50/p95/p99 for each phase separately; absent phases remain absent and partial phase coverage is rejected. Warmups run but do not enter results. Samples include units visited, actual hash bytes, upload/download bytes, requests, asset body reads/bytes, message visits and reused pages. Compaction overlap additionally requires catalog entries visited, peak temporary disk bytes and uncached input reads.

L1 supplies `cfg(test)` native candidate/hash counters. L2's `message_pages::Repaged.work` supplies message/page work, and the final driver records bytes actually hashed separately from input reads. Transport wrappers count bytes and requests at the provider or HTTP boundary. Do not infer native visits from outbox length, hash bytes from transfer size, or body reads from download counts.

`io_counters::IoCounters` supplies shared request and I/O counters for owned fake-provider drivers. Its download/body readers and upload writer count only successfully consumed bytes, including partial writes. The body wrapper counts acquisition even if no bytes are consumed, so a present-body restore cannot hide unnecessary opens. Requests are counted before each attempt, including failures. Use one fresh counter set per sample, acquire every provider/body stream through it, and sample only after all sample tasks have completed; a snapshot during concurrent transfer is not an atomic cross-field view.

Small Windows smoke uses the local Sync server and owned fake providers only, with the unshaped loopback profile. Final drivers must cover all `ALL_SCENARIOS`: routine changes, page insert/delete/oversized/sparse inputs, ordinary resume versus bootstrap, restore with present/missing bodies, consolidated/incomparable snapshots and compaction/backup/asset-transfer overlap. Restore-all-present samples reject any body read. Run that scenario at the target and above-target scales. `assert_bounded_work` compares identical changes across scales, using an explicit allowance decided before measurement. Latency variability does not excuse growth in visits/hash/transfer/request counts.

The final gate owns large measurements and all platform builds. No large measurement is run by the Wave 1 implementation. The proposed scheduling and pack values remain unmeasured until transport and maintenance drivers exercise them under a stated network profile.
