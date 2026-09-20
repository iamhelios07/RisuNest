# Tauri barcode scanner patch

This directory contains the RisuNest-pinned copy of Tauri's barcode scanner plugin. RisuNest uses it as a path dependency on Android and iOS.

The public JavaScript API remains the upstream Tauri plugin API. RisuNest-specific changes are kept narrow and tested with the application so they can be replayed onto future upstream releases.

## Project use

The plugin is registered by the RisuNest Tauri application. Do not install a second npm or crates.io copy when working in this repository.

Relevant checks:

```powershell
pnpm android:test
```

iOS compilation and runtime checks run through the repository's iOS verification workflow.

## Upstream and license

Upstream source: [tauri-apps/plugins-workspace](https://github.com/tauri-apps/plugins-workspace/tree/v2/plugins/barcode-scanner)

The plugin remains licensed under MIT or Apache-2.0, as declared in `Cargo.toml`.
