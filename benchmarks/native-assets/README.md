# Native asset measurements

With the desktop Rust prerequisites and workspace dependencies installed, run:

```text
node tests/native/run.mjs --asset-benchmark --debug --runs=3
```

This builds an agent harness and uses isolated synthetic profiles. Reports under
`.tmp/test-results/native/` contain the executable hash, WebView version, timings,
logical byte counts and transport calls. Each size has one warm-up and five
measured observations. Stored bytes and alias hashes are checked outside the
timed section. Reads immediately follow writes and use a warm filesystem cache.
The 64-asset workload measures direct writes, not import or restore.

For paired runs of already built executables, use the same build mode and add
`--reuse-build --binary=<absolute-executable-path>`. Keep each executable with
its source revision and changes. The runner hashes the selected executable but
cannot establish which source produced a reused build.

For small-transfer diagnosis, replace `--asset-benchmark` with
`--asset-benchmark-small`. This records one warm-up and 30 measured observations
at 0, 1 KiB, 64 KiB and 64 KiB + 1, with per-command timings and window
focus/visibility. Instrumented diagnostics are separate from the full workload
and must not replace its acceptance measurements.

On an owned synthetic Android emulator, build with
`pnpm benchmark:streaming:android:agent` and run the streaming smoke runner with
`--profile=asset-bytes` or `--profile=asset-bytes-legacy`. The latter disables the
optional Android binary bridge for that run. Both check concurrent DB, plugin
device-storage and asset writes after measurement.

For native backup restore with many assets, use the `assets` fixture axis in
`../legacy-restore/README.md`. These measurements are separate from ordinary
test gates and do not establish physical-device performance.
