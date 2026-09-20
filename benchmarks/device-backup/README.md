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

Read `docs/android-testing.md` first. Use the documented synthetic AVD, private ADB server, sustained health check, and agent APK.

```text
node benchmarks/device-backup/android-smoke.mjs --adb=<adb-executable> --apk=<synthetic-apk> --health=<health-summary.json> --output=<result.json>
```

Use `--exchange=<descriptor.json>` to publish a checked archive descriptor and `--peer=<windows-exchange.json>` to import one into the still-owned fixture.

A successful build is not a passing smoke test. The result must report successful assertions for the selected scenario. Never point either runner at an installed application, arbitrary device, user AVD, or user-provided backup.
