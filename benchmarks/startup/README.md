# Synthetic startup measurements

Measures startup, reload, interaction, storage, and memory behavior in an isolated Tauri application. It requires Node.js 24 or newer and uses only generated fixtures.

The Windows runner uses a separate application identifier, WebView profile, and verified native data directory. It runs on an unselected desktop unless `--display=foreground` is explicitly requested. Never point it at an installed application or real profile.

## Windows suite

Run from the repository root:

```powershell
node --test benchmarks/startup/startup.node-test.mjs
node benchmarks/startup/profile.mjs --suite=true --runs=20 --output=.tmp/startup-results.local.json
node benchmarks/startup/summarize.mjs .tmp/startup-results.local.json
```

The standard suite compares API 2.1 compatibility and scalable mode with 500 synthetic characters, about 100 MiB of logical data, generated images, and two CAS object counts. Each condition records its own warm-up.

Common single-condition options use `--name=value`:

| Option | Purpose |
| --- | --- |
| `characters`, `assets`, `bytes`, `runs` | Fixture and sample sizes |
| `mode=compatibility|scalable` | Working-set mode |
| `interact=true`, `images=true`, `mutation=true` | Timed UI and save work |
| `snapshot=missing`, `legacyAssets=N`, `referencedAssets=N` | Storage variants |
| `locale=ko`, `shape=investigation` | Presentation and fixture shape |
| `rebuild=true`, `fresh=true`, `hostMetrics=true` | Build, profile, and host observations |
| `kind=reload|restart` | Collect one complete sample group |

Use separate result files for each run. Do not combine partial groups or top up failed samples with selected successes.

## Android device runner

The physical-device runner requires an explicit ADB path, serial, agent APK, and 40-character source revision.

A first run uses `--provision=true --apk=<agent-apk>`. Later runs require the matching device, package, ownership marker, APK hash, and local record. Use `--prepare=true` to recreate synthetic data in that owned install.

For the optional emulator smoke, create a disposable synthetic AVD and use an owned local ADB server. Set `ANDROID_HOME` or pass `--adb=<adb-executable>`. The runner requires the exact emulator serial, AVD name, and ADB port, and rejects a different AVD:

```powershell
node benchmarks/startup/android-smoke.mjs --serial=<emulator-serial> --avd=<synthetic-avd-name> --adb-port=<owned-port> --apk=<agent-apk> --output=<local-json>
```

## Results

Output contains allowlisted metrics, fixed stage names, success flags, hashes, and build provenance. It excludes page text, request bodies, record identifiers, console events, and profile paths.

Timing begins at documented process or WebView boundaries. Heap and process memory are sampled observations, not allocation traces. Resource counters do not represent every disk read. Failed preparation or hidden-document samples do not count as measurements.
