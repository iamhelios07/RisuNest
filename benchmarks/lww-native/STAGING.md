# Local binding staging measurement

This ignored native test runs the product binding staging path with synthetic
assets, characters, conversation fields, and explicit ordering. Fixture creation
and source-body storage are outside the measured interval. It does not measure
network transfer, activation, or mobile-device performance.

Build a test executable in the repository's shared Cargo target directory:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --release --lib --locked --no-run
```

Run the resulting `risunest_lib-*.exe` with:

```powershell
$env:RISUNEST_STAGING_ASSETS = '1000'
$env:RISUNEST_STAGING_CHARACTERS = '100'
$env:RISUNEST_STAGING_CONVERSATIONS = '4'
$env:RISUNEST_STAGING_RECORD_BYTES = '256'
$env:RISUNEST_STAGING_TEXT_FIELDS = '0'
$env:RISUNEST_STAGING_MESSAGES_PER_CHAT = '0'
$env:RISUNEST_STAGING_MESSAGE_BYTES = '256'
$env:RISUNEST_STAGING_REPETITIONS = '3'
& $TestExecutable persistent_store::benchmark_lww::staging::measure_staging --ignored --exact --nocapture --test-threads=1
```

`CONVERSATIONS` is the count per character. `RECORD_BYTES` controls each
character's description size; larger descriptions use actual stored object
bodies. `TEXT_FIELDS` adds up to twelve small supported character text fields.
`MESSAGES_PER_CHAT` adds actual message-page bodies and manifests;
`MESSAGE_BYTES` controls each message's text size. Counts may be zero to isolate
an axis. An explicit character order is
always present, so the unit count can differ from the sum of fixture records.

Each `staging_measurement` JSON line contains input counts, elapsed milliseconds,
and phase observations with elapsed time and actual work counts. Phases run in
the order `read`, `validate`, `project`, `copy`, `check`. With N received units,
`validate` covers 2N steps (source hash and checks), `project` covers 4N (three
projection passes and the status hash; order refresh is timed but not counted),
and `copy` and `check` each cover the rows copied into the stage. `read` has a
null total. A retried request that reuses an existing stage leaves `validate`
after the source hash and reports one `check` phase with a null total. Phase
work is not the transfer item count or an overall completion percentage.

For shorter compilation during investigation, the build accepts
`--config 'profile.release.package.risunest.opt-level=1'`. Use identical compiler
and profile settings for a comparison and retain those settings with results.
