# Synthetic native device backup smoke

Exercises native device backup and restore with a separate agent build and synthetic data. Product builds never import this harness.

The baseline covers native Hypa, plugin-local values, device settings, WebView reload, restore, and cancellation. Fault mode terminates the owned synthetic process during activation and verifies startup recovery. Platform exchange mode restores an archive created by the other platform and compares checked fingerprints.

## Windows

```text
node benchmarks/device-backup/windows-smoke.mjs
node benchmarks/device-backup/windows-smoke.mjs --reuse=<baseline-result.json> --peer=<other-platform-exchange.json>
node benchmarks/device-backup/windows-smoke.mjs --reuse=<baseline-result.json> --fault=activating-database
```

The runner creates and verifies a unique native identifier, WebView profile, and app-data directory. Reuse only a baseline created by this harness.

## Android

Use a disposable synthetic AVD, an owned local ADB server, a sustained health-check result, and a separate agent APK. Set `ANDROID_HOME` or pass `--adb=<adb-executable>`. The runner requires the exact emulator serial, AVD name, and ADB port; it rejects a different AVD. SDK Build Tools 36.0.0 supplies `aapt`, or select it with `--aapt=<executable>`.

```text
node benchmarks/device-backup/android-smoke.mjs --serial=<emulator-serial> --avd=<synthetic-avd-name> --adb-port=<owned-port> --apk=<synthetic-apk> --health=<health-summary.json> --output=<result.json>
```

Use `--exchange=<descriptor.json>` to publish a checked archive descriptor and `--peer=<windows-exchange.json>` to import one into the still-owned fixture.

The health JSON must identify the same `Serial`, cover at least 120 `Seconds` and 8 `Samples`, report zero `Failures`, and set `Passed`, `HashMatch`, `Push`, `Pull`, and `ConsolePing` to true. Generate it from sustained ADB command, push/pull hash, and emulator-console checks; a result older than 30 minutes is rejected.

A successful build is not a passing smoke test. The result must report successful assertions for the selected scenario. Never point either runner at an installed application, arbitrary device, user AVD, or user-provided backup.
