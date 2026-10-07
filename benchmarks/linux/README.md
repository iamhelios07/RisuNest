# Linux native persistence benchmark

Runs production persistence and regex adapters, the product Svelte app with a synthetic character, and the streaming display suite in an isolated Linux Tauri application with synthetic fixtures. It does not read installed application data.

On Ubuntu with the Tauri dependencies, `WebKitWebDriver`, `xvfb`, and `dbus-run-session` installed:

```sh
export CARGO_TARGET_DIR=/absolute/path/to/RisuNest/src-tauri/target
pnpm tauri build --target x86_64-unknown-linux-gnu --no-bundle --config benchmarks/linux/tauri.conf.json
dbus-run-session -- xvfb-run -a python3 benchmarks/linux/run.py --binary "$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/release/RisuNest" --output .tmp/linux-benchmark --phase persistence
dbus-run-session -- xvfb-run -a python3 benchmarks/linux/run.py --binary "$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/release/RisuNest" --output .tmp/linux-benchmark --phase reload
dbus-run-session -- xvfb-run -a python3 benchmarks/linux/run.py --binary "$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/release/RisuNest" --output .tmp/linux-benchmark --phase regex
node benchmarks/linux/summarize.mjs .tmp/linux-benchmark
```

The `app`, `app-restart`, and `streaming` phases run the same way. `app` edits a message in the product chat and saves it, and `app-restart` reads it back after the process restarts.

Use the repository's existing shared Cargo target. Reuse the same output directory for the restart phases.

The runner creates and verifies its own XDG profile. Results include save latency, frame intervals, and sampled process-tree memory. They are Xvfb diagnostics, not physical Linux GPU, Wayland, IME, cold-start, or package compatibility evidence.
