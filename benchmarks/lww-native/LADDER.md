# LWW scaling ladder

## Commands

```powershell
# Smallest rungs, all phase groups
pwsh benchmarks/lww-native/ladder.ps1 -Rungs S1,S2

# Asset axis, record axis
pwsh benchmarks/lww-native/ladder.ps1 -Rungs A1k,A10k,A30k,A100k
pwsh benchmarks/lww-native/ladder.ps1 -Rungs R10,R50,R200

# One group, custom rung (name:assets:assetsPerOwner:recordBytes), reuse a built executable
pwsh benchmarks/lww-native/ladder.ps1 -Rungs X3k:3000:1000:5000000 -Groups backup -Executable <path-to-risunest_lib-*.exe>

# Build rungs only
pwsh benchmarks/lww-native/ladder.ps1 -Rungs A100k -ConstructOnly
```

The driver builds `cargo test --release --lib --no-run` in `src-tauri` with `RUSTUP_TOOLCHAIN=1.97.1` and `CARGO_TARGET_DIR=<main checkout>/src-tauri/target`, builds each missing rung once, then runs one child process per (rung, group).

| Named rung | Assets | Assets per owner | Record bytes |
|---|---|---|---|
| S1 / S2 | 1,000 / 2,000 | 1,000 | 5 MB |
| A1k / A10k / A30k / A100k | 1k / 10k / 30k / 100k | 10,000 | 20 MB |
| R10 / R50 / R200 / R1G | 5,000 | 1,000 | 10 / 50 / 200 MB / 1 GB |

## Driver parameters

| Parameter | Default |
|---|---|
| `-Rungs` | `S1` |
| `-Groups` | `server,backup,maintenance,serverless` |
| `-Output` | `<worktree>/.tmp/lww-ladder` (rungs in `rungs/<name>`, logs in `logs/`, scratch in `work/`) |
| `-Results` | `<Output>/results.jsonl` |
| `-Executable` | built by the driver |
| `-TempDirectory` | system temporary directory |
| `-Repetitions` / `-Warmups` | 5 / 1 (routine edits) |
| `-SampleMs` / `-PollMs` | 10 (in-process sampler) / 250 (driver poll) |
| `-CargoArgs` | none; appended to the build command |
| `-ConstructOnly`, `-KeepWork` | off |

## Entry tests

`persistent_store::benchmark_lww::ladder::ladder_construct_rung` and `persistent_store::benchmark_lww::ladder::ladder_run_group`, both `#[ignore]`, run with `--ignored --exact --nocapture --test-threads=1`.

| Variable | Test | Default |
|---|---|---|
| `RISUNEST_LADDER_RUNG_DIR` | both | required, absolute; new or empty for construction |
| `RISUNEST_LADDER_NAME` | construct | rung directory name |
| `RISUNEST_LADDER_ASSETS` | construct | required |
| `RISUNEST_LADDER_ASSETS_PER_OWNER` | construct | 1000 |
| `RISUNEST_LADDER_RECORD_BYTES` | construct | required |
| `RISUNEST_LADDER_MESSAGES_PER_CHARACTER` | construct | 256 (character 0 always has 4,096) |
| `RISUNEST_LADDER_SEED` | construct | 1 |
| `RISUNEST_LADDER_GROUP` | run | required: `server`, `backup`, `maintenance`, `serverless` |
| `RISUNEST_LADDER_OUTPUT_DIR` | run | `<worktree>/.tmp/lww-ladder` |
| `RISUNEST_LADDER_RESULTS` | run | `<output>/results.jsonl` |
| `RISUNEST_LADDER_REPETITIONS` / `RISUNEST_LADDER_WARMUPS` | run | 5 / 1 |
| `RISUNEST_LADDER_SAMPLE_MS` | run | 10 |
| `RISUNEST_LADDER_TEMP_DIR` | run | unset (system temporary directory); keep under about 80 characters on Windows |
| `RISUNEST_LADDER_KEEP_WORK` | run | unset; `1` keeps `<output>/work/<rung>-<group>-<ms>` |

## Phases

| Group | Phases |
|---|---|
| server | `initial-publication`, `source-receive-after-publication`, `bootstrap-remote`, `residency-status-remote`, `bootstrap-full`, `peer-receive-after-bootstrap`, `routine-setting`, `routine-append`, `routine-middle-edit`, `routine-burst` |
| backup | `device-backup`, `restore-all-present`, `restore-missing-1pct` |
| maintenance | `residency-status`, `storage-stats`, `cleanup-preview`, `cleanup-execute` (1% unreferenced bodies injected during setup) |
| serverless | `serverless-publish`, `serverless-bootstrap` |

## Output

`rungs/<name>/rung.json` (`risunest.lww-ladder-rung/v1`): `params`, `counts` (characters, conversations, messages, `activeRecordBytes`, aliases, owner heads, catalog objects and bytes), `shards`, `files` (SQLite sizes), `constructionMs`, `binary`.

`results.jsonl`, one line per phase (`risunest.lww-ladder/v1`):

| Field | Content |
|---|---|
| `rung`, `group`, `run`, `pid`, `tempDir`, `phase` | identity |
| `status`, `error` | `ok`, `failed`, or `skipped` (a phase it depends on did not complete) |
| `elapsedMs` | measured window; routine phases report the median end-to-end receive time |
| `subTimingsMs` | phase-specific splits |
| `memory` | `workingSetAfterSetup`, `privateBytesAfterSetup`, `peakWorkingSet`, `peakPrivateBytes` (sampler), `processPeakWorkingSet`, `processPeakPrivateBytes`, `samples`, `processIo` |
| `setup` | fork or seed work excluded from `elapsedMs` |
| `counts` | phase counts, HTTP or provider counters, `observers` (hash domains as `[calls, bytes]`, body I/O per domain, `observerRows`) |
| `samples` | routine phases only, one entry per iteration |
| `sqlite` | `persistent`, `device` and WAL sizes after the phase |
| `phaseWallMs` | wall time including setup |

One line per child process (`risunest.lww-ladder-process/v1`): `rung`, `group` (`construct` for rung construction), `pid`, `exitCode`, `wallMs`, `peakWorkingSet`, `peakPrivateBytes`, polled values, `log`.

## Sync server source process

The final matrix and the ignored `actual_source_*` tests in `persistent_store::benchmark_lww` start the Sync server library test binary named by `LWW_SOURCE_TEST_EXE` and drive its `source_observer_harness::serve` test. The observer compiles only into Sync server test builds with the `source-observer` feature; a binary built without it has no `serve` test. Build it from the repository root with the same toolchain and shared target as the driver:

```powershell
$env:RUSTUP_TOOLCHAIN = '1.97.1'
$env:CARGO_TARGET_DIR = '<main checkout>/src-tauri/target'
cargo test --manifest-path server/sync/Cargo.toml --release --locked --lib --no-run --features source-observer
```

`LWW_SOURCE_TEST_EXE` is the `unittests src/lib.rs` executable Cargo prints, and `LWW_SOURCE_BINARY_SHA` is its SHA-256. `serve` refuses a `LWW_SOURCE_FINGERPRINT` that differs from the source identity compiled into it.
