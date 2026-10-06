# Tauri updater patch

This directory contains the RisuNest-pinned desktop updater plugin. RisuNest uses it as a path dependency on Windows, macOS, and Linux.

The public JavaScript API remains the upstream Tauri updater API. Local changes support RisuNest's signed release and platform update behavior and are kept inside this patched dependency.

## Project use

The plugin is registered by the RisuNest Tauri application. Do not add a second crates.io copy for desktop targets.

Run the updater and release checks from the repository root:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --package tauri-plugin-updater --locked --lib
pnpm test:release
```

The tests run through the application manifest so they build with the application's features. Cargo cannot test a non-member package that has dev-dependencies, so the upstream `tauri` dev-dependency, which only the documentation examples use, is removed from `Cargo.toml` and `Cargo.toml.orig`.

## Upstream and license

Upstream source: [tauri-apps/plugins-workspace](https://github.com/tauri-apps/plugins-workspace/tree/v2/plugins/updater)

The plugin remains licensed under MIT or Apache-2.0, as declared in `Cargo.toml`.
