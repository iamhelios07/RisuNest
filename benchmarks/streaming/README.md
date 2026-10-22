# Android streaming display smoke

Mounts the production chat display components with synthetic snapshots and exercises CBS, display regex, Markdown, sanitization, and optional persistence. The harness has its own agent-mode frontend and is never imported by the product entry.

Use a disposable synthetic AVD, an owned local ADB server, an agent APK, and a shared Cargo target. The runner verifies the selected AVD and package before use and leaves the synthetic AVD offline. On Android 13 and later, it grants and checks notification permission for the synthetic package before launch.

## Build and run

Set the installed JDK, Android SDK, NDK, and shared Cargo target in the environment, then run:

```powershell
pnpm benchmark:streaming:android:agent
node benchmarks/streaming/android-smoke.mjs --serial=<emulator-serial> --avd=<synthetic-avd-name> --adb-port=<owned-port>
```

Set `ANDROID_HOME` or pass `--adb=<adb-executable>`. Pass the exact emulator serial, AVD name, and ADB port on every run. The runner uses that local server even when the shell has different ADB settings, and rejects a different AVD.

## Profiles

| Profile | Purpose |
| --- | --- |
| `smoke` | Display modes, Thoughts handling, finalization, regex, source preservation, and publication cadence |
| `stress` | Reproduces the large expanded-Thought renderer stall |
| `persistence-spike` | Compares bounded JSON, byte-array, and string IPC returns |
| `persistence` | Exercises the production Worker encoder and Android commit adapter |
| `parser-context` | Measures a character switch in the mounted production chat screen with synthetic assets |
| `asset-bytes` | Measures native asset and inlay transfers and verifies concurrent DB/device/asset writes |
| `asset-bytes-legacy` | Runs the same asset checks with the optional Android binary bridge disabled |
| `regex-time-limit` | Measures 30 rendered rows at 0/50/200/500 display rules, 2 KB/30 KB inputs, and subsequent row updates |

For a Windows before/after comparison, serve each revision with the agent streaming harness and run
`node benchmarks/streaming/regex-time-limit.mjs <before-loopback-url> <after-loopback-url> <output-json>`.
The same synthetic fixture must be present in both revisions. The after server also checks an 8,000-character
unclosed regex input with the default timeouts, a concurrent UI timer, and repeat rendering.

Examples:

```powershell
node benchmarks/streaming/android-smoke.mjs --serial=<emulator-serial> --avd=<synthetic-avd-name> --adb-port=<owned-port> --profile=stress --output=benchmarks/streaming/android-stress.local.json
node benchmarks/streaming/android-smoke.mjs --serial=<emulator-serial> --avd=<synthetic-avd-name> --adb-port=<owned-port> --profile=persistence --output=benchmarks/streaming/persistence-result.local.json
node benchmarks/streaming/android-smoke.mjs --serial=<emulator-serial> --avd=<synthetic-avd-name> --adb-port=<owned-port> --profile=parser-context --assets=4000 --baseline=false --output=benchmarks/streaming/parser-context-result.local.json
```

The persistence and parser-context profiles replace the native store with a tracked synthetic fixture. Never run them on another device or profile. `--fresh-install=true` is valid only after the runner verifies ownership of the selected synthetic AVD and package.

For parser-context, `--assets` selects the synthetic asset count and `--baseline=true` measures the former per-row context copy. Completion requires the expected conversation, prepared message rows, and stable ordered row identities and work counts across three frames. Compare switch timings only when the settled ordinary message indices, message IDs, and per-index parse counts match; row counts alone are insufficient. Render keys establish stability within each run and may differ between runs. The result measures the mounted component, not full application bootstrap. Observed runtime exceptions, including unhandled promise rejections reported by CDP, fail the measurement; counts cover the CDP attachment onward, not earlier startup.

Results contain environment identity, APK hash, assertion IDs, synthetic conversation and row identities, booleans, counts, and timing metrics. They do not contain console events, exception contents, network bodies, or fixture text.

These runs validate display and persistence paths, not provider transport, full application bootstrap, real plugins, touch behavior, background generation, or physical-device performance.

For the regex timeout responsiveness check on an Android build with timeout handling, add `--timeout=true` to `--profile=regex-time-limit`. The Windows regex driver also records 1/10/30-row scaling and retained UI heap after five repeated 200-rule runs; Worker input retention is covered by the protocol tests.
