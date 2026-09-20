# Roadmap 14 platform baseline

Defines the shared result schema and four synthetic scenarios used by Windows and Android release measurements. Runners use agent mode and block RisuRealm.

## Contract

- `result.schema.json` defines the result format.
- `result-schema.mjs` validates runner output.
- `scenarios.json` defines `library-many`, `save-large`, `stream-postprocess`, and `asset-library`.
- Completed results require build and fixture provenance, memory and latency samples, UI counts, artifact hashes, and a canonical output hash.

Run the focused contract tests with:

```powershell
node --test benchmarks/roadmap14/result-schema.node-test.mjs benchmarks/phase3/tauri-cdp.node-test.mjs
```

## Windows

Run the existing release measurements and convert them to the shared schema:

```powershell
node benchmarks/roadmap14/windows.mjs `
  --run-existing `
  --platform-identity windows-reference-host `
  --output src-tauri/target/roadmap14-baseline/windows-save-large.json
```

Convert existing raw results without rerunning:

```powershell
node benchmarks/roadmap14/windows.mjs `
  --phase3-result <phase3-save-large.json> `
  --tauri-result <phase3-tauri-cdp.json> `
  --platform-identity windows-reference-host `
  --output <windows-save-large.json>
```

The converter rejects mismatched revisions, application identifiers, fixtures, or raw artifact hashes.

## Android

Generate an explicit pending record when no physical-device result exists:

```powershell
node benchmarks/roadmap14/android.mjs `
  --source-revision <revision> `
  --output src-tauri/target/roadmap14-baseline/android-pending.json
```

Validate completed physical-device output with:

```powershell
node benchmarks/roadmap14/android.mjs `
  --measurement <physical-device-results.json> `
  --output <validated-results.json>
```

An APK build, JVM test, emulator run, or pending record is not physical-device performance evidence. Scenario definitions and acceptance rationale live under `docs/native-app-analysis/roadmap-14-implementation/`.
