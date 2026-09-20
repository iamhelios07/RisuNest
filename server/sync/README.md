# RisuNest Sync

RisuNest Sync is a standalone daemon for synchronizing a RisuNest library between devices. It runs without Tauri, a WebView, Node.js, providers, or plugins.

This project has not shipped a stable release yet. Use synthetic data while developing or validating it.

## Choose a distribution

- The managed distribution includes the daemon, management app, updater, and `cloudflared`.
- The raw distribution contains only `risunest-sync-server` and is intended for manual administration.

The managed distribution is recommended for normal use. Its GUI or TUI configures devices, startup, connections, and updates without editing the server database directly.

## Raw daemon quick start

Choose a new absolute data directory. The daemon refuses an existing incompatible store, symlink, or junction.

### Windows

```powershell
$data = Join-Path $env:LOCALAPPDATA 'RisuNestSyncData'
./risunest-sync-server.exe init --data-dir $data
./risunest-sync-server.exe device add --data-dir $data
./risunest-sync-server.exe serve --data-dir $data
```

### Linux

```sh
data="${XDG_DATA_HOME:-$HOME/.local/share}/risunest-sync"
./risunest-sync-server init --data-dir "$data"
./risunest-sync-server device add --data-dir "$data"
./risunest-sync-server serve --data-dir "$data"
```

### macOS

```sh
data="$HOME/Library/Application Support/io.github.rsyumi.risunest.sync-manager"
./risunest-sync-server init --data-dir "$data"
./risunest-sync-server device add --data-dir "$data"
./risunest-sync-server serve --data-dir "$data"
```

`device add` prints a library ID, device ID, and token once. Transfer the credential privately to the intended device. The server stores only its verifier.

In RisuNest, open the server synchronization settings and enter the values printed by the daemon. `http://127.0.0.1:14319` works only on the server machine.

Run `risunest-sync-server --help` or a subcommand with `--help` for the complete CLI reference.

## Registration and remote access

Configure a fixed public HTTPS address while the daemon is stopped:

```powershell
./risunest-sync-server.exe connection configure --data-dir $data --endpoint https://sync.example
./risunest-sync-server.exe device add --data-dir $data --qr
```

Add `--registry https://registry.example` when the address should be discovered through an endpoint registry. The registry receives only an encrypted address, never the device token or decryption key.

For a daemon-owned Quick Tunnel, use `--cloudflared ABSOLUTE_EXECUTABLE` instead of `--endpoint`. Quick Tunnel addresses may change after restart, so configure a registry when registration must remain reusable. For stable remote access, prefer a named Tunnel or another trusted HTTPS reverse proxy.

The `--https-proxy` option acknowledges an existing HTTPS proxy configuration. It does not install a proxy or add TLS to a public HTTP listener.

Treat the complete registration URI or QR code as a reusable device credential. Revoke the device if the credential is lost or exposed.

## Administration

Stop the daemon before running commands that modify its store, including:

```text
status
device add
device revoke ID
connection configure
connection repost
maintain
backup
restore
restore-epoch
```

The daemon uses an exclusive owner lock. A second daemon or administrative process fails rather than replacing live data.

Run `maintain` to collect history and objects that are no longer retained by active devices. Offline devices remain active until explicitly revoked.

## Backup and restore

Stop the daemon and use new absolute destination paths:

```powershell
./risunest-sync-server.exe backup --data-dir $data --backup-dir E:/backups/risunest-sync
./risunest-sync-server.exe restore --backup-dir E:/backups/risunest-sync --data-dir E:/restored/risunest-sync
```

Backup verifies registered objects and writes `backup.json` only after completion. A directory without that completion marker is not a valid backup. Restore verifies the backup, creates a new server epoch, and preserves registered devices so clients can reconcile with the restored server.

The private `connection-state` file is operational configuration and is not part of a library backup. Preserve it separately under the same daemon OS account when restoring the same registry identity.

## Updates

Managed installations support automatic, notification-only, and disabled update policies through the management app.

The raw daemon provides a read-only check:

```powershell
./risunest-sync-server.exe update check
```

It verifies the signed release catalog and reports the matching raw package. It does not download or install anything.

## Build and verify from source

Use the repository's shared Cargo target directory:

```powershell
$env:CARGO_TARGET_DIR = (Resolve-Path 'src-tauri/target').Path
cargo test --manifest-path crates/sync-wire/Cargo.toml --locked
cargo test --manifest-path server/sync/Cargo.toml --locked
node --test crates/sync-wire/tests/golden.test.mjs
cargo clippy --manifest-path server/sync/Cargo.toml --locked --all-targets -- -D warnings
```

Architecture, protocol, and verification records belong under `docs/`, especially:

- `docs/superpowers/specs/2026-09-08-lightweight-sync-server-design.md`
- `docs/superpowers/specs/2026-09-12-server-only-sync-design.md`
- `docs/research/lightweight-sync-server-final-verification-2026-09-12.md`
- `docs/research/2026-09-12-sync-server-benchmark-results.md`
