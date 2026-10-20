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

## Gemma 4

The application's Gemma Vitest tests use the real WASM engine and read the
bundled, minified tokenizer from
[`public/token/gemma`](../../public/token/gemma/SOURCE.md), which includes its
license and source hashes. External requests are rejected by the test boundary.
Reference IDs are stored in `tests/fixtures/tokenizer/gemma4.json`.

Verify the expected IDs independently with Python 3.9 or later:

```sh
python -m pip install tokenizers==0.22.2
python benchmarks/tokenizer/verifyGemmaTokenizer.py
```

This check validates the bundled file's SHA-256 and reference IDs without network
requests or fixture edits. Python is not required for the regular Vitest suite.

Measure the bundled JSON's compressed sizes locally:

```sh
node benchmarks/tokenizer/gemmaArtifactSizes.mjs
```

This checks the bundled file's hash and minified form, then reports raw, gzip and
Brotli sizes at qualities 2, 9 and 11 using Node's codecs. These are artifact sizes,
not measured installer or APK size changes. It does not modify product assets.
