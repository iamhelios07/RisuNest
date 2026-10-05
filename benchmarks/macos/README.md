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
6. a quit repeated while the first one waits ends the application without asking the product again

A failed phase skips only the phases that read the data it leaves: restart and the product flow after the contracts, and application restart after the product flow. The other phases still run, and the controller reports every failure at the end.

Before a remote macOS run, check locally what would otherwise fail only there: harness type errors, commands the harness invokes that no handler registers, and a harness lockfile or capability that no longer matches the product.

```sh
pnpm check:benchmark-harnesses
test -f dist/index.html || { mkdir -p dist && printf '<!doctype html>\n' > dist/index.html; }
test -f benchmarks/macos/dist/index.html || { mkdir -p benchmarks/macos/dist && printf '<!doctype html>\n' > benchmarks/macos/dist/index.html; }
node benchmarks/macos/prepare.mjs
cargo check --locked --manifest-path benchmarks/macos/native/Cargo.toml
```

Reports contain synthetic assertions, timings, hashes, and process observations. Hosted-VM frame and memory measurements are diagnostics, not physical-device responsiveness, power, total memory, signing, or notarization evidence.
