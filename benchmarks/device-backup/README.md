# Synthetic native device backup smoke

Exercises native device backup and restore with a separate agent build and synthetic data. Product builds never import this harness.

The baseline covers native Hypa, plugin-local values, device settings, WebView reload, restore, and cancellation. Fault mode terminates the owned synthetic process during activation and verifies startup recovery. Platform exchange mode restores an archive created by the other platform and compares checked fingerprints.

## Windows

```text
node benchmarks/device-backup/windows-smoke.mjs
node benchmarks/device-backup/windows-smoke.mjs --reuse=<baseline-result.json> --peer=<other-platform-exchange.json>
node benchmarks/device-backup/windows-smoke.mjs --reuse=<baseline-result.json> --fault=activating-database
```

The runner creates a named agent profile and verifies its native identifier, WebView profile, and resolved app-data directory before initializing data. Reuse only a baseline created by this harness.

## Android

Use a disposable synthetic AVD, an owned local ADB server, a sustained health-check result, and a separate agent APK. Set `ANDROID_HOME` or pass `--adb=<adb-executable>`. The runner requires the exact emulator serial, AVD name, and ADB port; it rejects a different AVD. SDK Build Tools 36.0.0 supplies `aapt`, or select it with `--aapt=<executable>`.

```text
node benchmarks/device-backup/android-smoke.mjs --serial=<emulator-serial> --avd=<synthetic-avd-name> --adb-port=<owned-port> --apk=<synthetic-apk> --health=<health-summary.json> --output=<result.json>
```

Use `--exchange=<descriptor.json>` to publish a checked archive descriptor and `--peer=<windows-exchange.json>` to import one into the still-owned fixture.

The health JSON must identify the same `Serial`, cover at least 120 `Seconds` and 8 `Samples`, report zero `Failures`, and set `Passed`, `HashMatch`, `Push`, `Pull`, and `ConsolePing` to true. Generate it from sustained ADB command, push/pull hash, and emulator-console checks; a result older than 30 minutes is rejected.

A successful build is not a passing smoke test. The result must report successful assertions for the selected scenario. Never point either runner at an installed application, arbitrary device, user AVD, or user-provided backup.

## Large plugin values

The plugin measurement writes and reads synthetic UTF-8 string and JSON values, backs them up and restores them, and synchronizes them between fresh profiles through a private loopback Sync server. JSON cases use flat strings. Results include exact payload hashes, elapsed times, renderer heap samples, and native memory peaks. The Windows runner also records app and WebView process working sets. The payload sizes count the UTF-8 bytes of the JSON-encoded stored string.

```text
node benchmarks/device-backup/windows-smoke.mjs --plugin-values=all
node benchmarks/device-backup/android-smoke.mjs --serial=<emulator-serial> --avd=<synthetic-avd-name> --adb-port=<owned-port> --apk=<synthetic-apk> --health=<health-summary.json> --output=<result.json> --plugin-values=64-string --sync-server=<sync-server-executable>
```

Windows accepts `all`, `json` (all three JSON sizes), or one of `64-string`, `64-json`, `256-string`, `256-json`, `512-string`, and `512-json`. Set `RISUNEST_PLUGIN_SYNC_SERVER` to select the server executable; the default is the shared Cargo target's release binary. Android takes one case per run and requires `--sync-server`; it clears only the validated disposable AVD's synthetic application's data between peers. Its loopback server uses ADB reverse while device networking remains disabled. Registration credentials stay in private fixture files and are excluded from reports. Keep those files and synthetic profile directories outside published artifacts.
