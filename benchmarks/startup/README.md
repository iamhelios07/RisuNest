# Synthetic startup measurements

Measures startup, reload, interaction, storage, and memory behavior in an isolated Tauri application. It requires Node.js 24 or newer and uses only generated fixtures.

The Windows runner uses a separate application identifier, WebView profile, and verified native data directory. It runs on an unselected desktop unless `--display=foreground` is explicitly requested. Never point it at an installed application or real profile.

## Windows suite

Run from the repository root:

```powershell
node --test benchmarks/startup/startup.node-test.mjs
node benchmarks/startup/profile.mjs --suite=true --runs=20 --output=docs/research/startup-results.local.json
node benchmarks/startup/summarize.mjs docs/research/startup-results.local.json
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

Read `docs/android-testing.md` before use. The physical-device runner requires an explicit ADB path, serial, agent APK, and 40-character source revision.

A first run uses `--provision=true --apk=<agent-apk>`. Later runs require the matching device, package, ownership marker, APK hash, and local record. Use `--prepare=true` to recreate synthetic data in that owned install.

The optional emulator smoke is restricted to its documented synthetic AVD and private ADB server:

```powershell
node benchmarks/startup/android-smoke.mjs --adb=<adb-executable> --output=<local-json>
```

## Results

Output contains allowlisted metrics, fixed stage names, success flags, hashes, and build provenance. It excludes page text, request bodies, record identifiers, console events, and profile paths.

Timing begins at documented process or WebView boundaries. Heap and process memory are sampled observations, not allocation traces. Resource counters do not represent every disk read. Failed preparation or hidden-document samples do not count as measurements.

Design decisions and completed investigations are recorded in:

- `docs/superpowers/specs/2026-09-08-large-library-startup-design.md`
- `docs/research/large-library-startup-implementation-2026-09-09.md`
- `docs/research/large-library-startup-final-verification-2026-09-09.md`
