# Tokenizer compatibility and performance

Contains the literal JavaScript oracle corpus, standalone oracle benchmark, and isolated Windows Tauri comparison for the current cl100k and o200k artifacts.

## Corpus

`native-tokenizer-corpus.json` contains exact token-ID and disallowed-special-token cases, Unicode boundaries, control inputs, and artifact hashes. Expected values are checked in and are never derived from the native implementation.

```powershell
pnpm test:tokenizer-corpus
```

`generate-corpus.mjs` emits an `apply_patch` block so corpus updates remain explicit review steps.

## JavaScript oracle

```powershell
pnpm benchmark:tokenizer:oracle -- --samples 20 --output tokenizer-oracle.json
```

This verifies the corpus and records count and ID timings for deterministic batches. It is a JavaScript baseline, not a native comparison.

## Windows Tauri comparison

Use the repository's shared Cargo target:

```powershell
$env:CARGO_TARGET_DIR = (Resolve-Path 'src-tauri/target').Path
pnpm benchmark:tokenizer:tauri -- --output "$env:CARGO_TARGET_DIR/k2-tokenizer-windows.json"
```

The runner builds an isolated agent-mode Tauri application, verifies both implementations against the checked-in corpus, and times them in the same release WebView. It owns its application and profile and removes them unless a completed run uses `--keep-profile`.

Measure the Rust core without IPC separately:

```powershell
pnpm benchmark:tokenizer:core
```

The harness does not access RisuRealm or account services.
