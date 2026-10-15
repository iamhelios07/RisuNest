# Synthetic Sync server checks

Checks synthetic transfers, TLS, and Android ADB health. The harnesses never read installed profiles or retain credentials, page content, or request bodies.

## Android ADB health

Use a disposable synthetic AVD and an owned local ADB server. Set `ANDROID_HOME` or pass `--adb=<adb-executable>`. The probe requires the exact emulator serial, AVD name, and ADB port, and rejects a different AVD.

    node benchmarks/sync-server/android-probe.mjs --serial=<emulator-serial> --avd=<synthetic-avd-name> --adb-port=<owned-port>

The probe checks sustained ADB transfer and health. Current application flows are covered by [Sync UI](../sync-ui/README.md) and [device backup](../device-backup/README.md) harnesses.

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

Harness output stays under ignored `.local` paths.
