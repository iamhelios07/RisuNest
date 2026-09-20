# Synthetic Sync server checks

Runs isolated Android and Windows applications against synthetic Sync daemons. The harnesses never read installed profiles or retain credentials, page content, or request bodies.

## Android

Read docs/android-testing.md first. Use the documented risunest_vm_retest AVD, private ADB port 15037, agent build, and isolated application ID.

After the agent build, create the isolated APK from src-tauri/gen/android:

    ./gradlew.bat :app:assembleX86_64Debug -x :app:rustBuildX86_64Debug
      '-Pkotlin.incremental=false' '-Pkotlin.compiler.execution.strategy=in-process'
      -I '../../../benchmarks/sync-server/android-profile.gradle' --offline

Verify its package with aapt dump badging, then run from the repository root:

    node benchmarks/sync-server/android-probe.mjs
    node benchmarks/sync-server/android-runtime.mjs
    node benchmarks/sync-server/android-runtime.mjs --ui
    node benchmarks/sync-server/android-runtime.mjs --unresponsive-startup

The probe checks sustained ADB transfer and health. The runtime checks native IPC, Keystore recovery, registration, synchronization, restart, progress UI, and the bounded unresponsive-server case. They do not cover an Android foreground service.

## Windows

Build the application with its isolated Tauri identifier and run:

    $env:CARGO_TARGET_DIR = (Resolve-Path 'src-tauri/target').Path
    pnpm exec tauri build --debug --no-bundle --config src-tauri/tauri.agent.conf.json --config benchmarks/sync-server/windows-validation.conf.json
    node benchmarks/sync-server/windows-runtime.mjs

The launcher verifies the executable, identifier, process, profile, and CDP listener before registering a synthetic daemon.

## Transfer and TLS measurements

Build the standalone release daemon first:

    cargo build --manifest-path server/sync/Cargo.toml --release --locked
    node benchmarks/sync-server/transfer-comparison.mjs
    $env:OPENSSL_EXE = '<path-to-openssl>'
    node benchmarks/sync-server/tls-head.mjs

The transfer comparison checks individual and batched full-object transfers, exact hashes, framing, worker counts, and counted HTTP bytes. The TLS probe uses an ephemeral self-signed fixture certificate and does not modify OS trust.

Run codec checks without a daemon:

    node --test benchmarks/sync-server/transfer-comparison.test.mjs
    cargo test --manifest-path crates/sync-wire/Cargo.toml --test golden --locked

Set RISUNEST_SYNC_SERVER_BINARY to test a specific existing daemon. Debug-daemon timing is correctness evidence only.

## Explicit scale gates

These ignored tests can take substantial time and disk IO:

    cargo test --manifest-path src-tauri/Cargo.toml --locked --lib server_sync_large_value_and_sequence_http_matrix -- --ignored --nocapture
    cargo test --manifest-path src-tauri/Cargo.toml --locked --lib slow_network_full_transfer_gate -- --ignored --nocapture
    cargo test --manifest-path src-tauri/Cargo.toml --locked --lib server_sync_local_commit_and_large_json_costs -- --ignored --nocapture
    cargo test --manifest-path src-tauri/Cargo.toml --locked --lib server_sync_hundred_thousand_resolved_assets_http_gate -- --ignored --nocapture

Harness output stays under ignored .local paths. Current verification is recorded in docs/research/lightweight-sync-server-final-verification-2026-09-12.md; the measurements removed from this README are preserved in docs/research/2026-09-12-sync-server-benchmark-results.md.
