# macOS desktop verification

Runs WKWebView and the product Rust setup, command router, storage, and macOS event handling in an isolated application with synthetic fixtures. Product builds do not import the harness.

Use a fresh CI user profile:

```sh
pnpm install --frozen-lockfile
pnpm build:agent
pnpm exec vite build --mode agent --config benchmarks/macos/vite.config.ts
node benchmarks/macos/prepare.mjs
export CARGO_TARGET_DIR="$PWD/src-tauri/target"
export APPLE_SIGNING_IDENTITY=-
(cd benchmarks/macos/native && node ../../../node_modules/@tauri-apps/cli/tauri.js build --ci --bundles app -- --locked)
python3 benchmarks/macos/run.py \
  --app "$CARGO_TARGET_DIR/release/bundle/macos/RisuNest Mac Bench.app" \
  --artifacts artifacts/wkwebview
```

Use the project-mandated shared Cargo target when running from a worktree.

The controller verifies:

1. persistence, tokenizer, regex, file-open, and quit contracts
2. restart persistence
3. the ordinary Svelte product flow with a synthetic character
4. application restart
5. the existing streaming display suite

Reports contain synthetic assertions, timings, hashes, and process observations. Hosted-VM frame and memory measurements are diagnostics, not physical-device responsiveness, power, total memory, signing, or notarization evidence.
