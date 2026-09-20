# Tauri updater patch

This directory contains the RisuNest-pinned desktop updater plugin. RisuNest uses it as a path dependency on Windows, macOS, and Linux.

The public JavaScript API remains the upstream Tauri updater API. Local changes support RisuNest's signed release and platform update behavior and are kept inside this patched dependency.

## Project use

The plugin is registered by the RisuNest Tauri application. Do not add a second crates.io copy for desktop targets.

Run the updater and release checks from the repository root:

```powershell
cargo test --manifest-path crates/tauri-plugin-updater/Cargo.toml --locked
pnpm test:release
```

## Upstream and license

Upstream source: [tauri-apps/plugins-workspace](https://github.com/tauri-apps/plugins-workspace/tree/v2/plugins/updater)

The plugin remains licensed under MIT or Apache-2.0, as declared in `Cargo.toml`.
