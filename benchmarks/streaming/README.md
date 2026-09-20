# Android streaming display smoke

Mounts the production chat display components with synthetic snapshots and exercises CBS, display regex, Markdown, sanitization, and optional persistence. The harness has its own agent-mode frontend and is never imported by the product entry.

Read `docs/android-testing.md` first. Use only the documented synthetic AVD, private ADB server, agent APK, and shared Cargo target. The runner verifies the AVD and package before use and leaves the synthetic AVD offline.

## Build and run

Set the installed JDK, Android SDK, NDK, and shared Cargo target in the environment, then run:

```powershell
pnpm benchmark:streaming:android:agent
$adb = Join-Path $env:ANDROID_HOME 'platform-tools/adb.exe'
node benchmarks/streaming/android-smoke.mjs --adb=$adb
```

Use the documented `ADB_SERVER_SOCKET` and `ANDROID_ADB_SERVER_PORT=15037` when the default port is reserved.

## Profiles

| Profile | Purpose |
| --- | --- |
| `smoke` | Display modes, Thoughts handling, finalization, regex, source preservation, and publication cadence |
| `stress` | Reproduces the large expanded-Thought renderer stall |
| `persistence-spike` | Compares bounded JSON, byte-array, and string IPC returns |
| `persistence` | Exercises the production Worker encoder and Android commit adapter |

Examples:

```powershell
node benchmarks/streaming/android-smoke.mjs --adb=$adb --profile=stress --output=benchmarks/streaming/android-stress.local.json
node benchmarks/streaming/android-smoke.mjs --adb=$adb --profile=persistence --output=benchmarks/streaming/persistence-result.local.json
```

The persistence profile replaces the native store with a tracked synthetic fixture. Never run it on another device or profile. `--fresh-install=true` is valid only after the runner verifies ownership of the selected synthetic AVD and package.

Results contain environment identity, APK hash, assertion IDs, booleans, counts, and timing metrics. They do not contain console events, network bodies, or fixture values.

These runs validate display and persistence paths, not provider transport, full application bootstrap, real plugins, touch behavior, background generation, or physical-device performance. Detailed design and completed measurements live in:

- `docs/superpowers/specs/2026-09-10-android-streaming-smoke-design.md`
- `docs/research/android-streaming-smoke-2026-09-10.md`
- `docs/research/android-streaming-ipc-followup-2026-09-10.md`
