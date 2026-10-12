# AGENTS.md

RisuNest is a cross-platform AI chat application, a fork of RisuAI focused on large-library performance and native app targets. Stack: Svelte 5 + TypeScript, Tauri 2 (Rust backend; generated Android project in `src-tauri/gen/android`), Vite 8, Tailwind CSS 4, pnpm. It lets users chat with many AI models through one interface, with themes, plugins, custom assets, and advanced memory systems.

Platform targets: Windows, macOS, Linux, Android, and iOS, all equal. A feature, and the optimization behind it, belongs on all five; when a task cannot build one platform's equivalent, record the gap instead of letting the platforms diverge. Every target keeps the Svelte WebView UI; a native Rust data core or native UI is adopted only when measurements justify it. The web build only has to stay error-free: a feature that is easy natively but unavailable on the web is skipped, disabled, or guarded there, never worked around and never held back for web parity.

## Fork Scope

- Keep upstream RisuAI features and options in their original settings locations. The RisuNest settings tab is reserved for RisuNest-added features and options; never move upstream controls into it.
- Provider-specific request formatting, SSE decoders, and response parsers follow upstream RisuAI. Do not refactor them locally except for correctness fixes.
- Optimization targets are the provider-independent paths: regex scripts, lorebook, Lua, stream postprocessing, rendering, storage, and sync.
- Prefer narrow platform adapters and compatibility seams that minimize conflicts with upstream RisuAI.

## Layout

- `src/ts/` TypeScript logic with tests colocated as `*.test.ts`; `src/lib/` Svelte components; `src/lang/` i18n; `src-tauri/` Rust backend and the generated Android shell; `crates/` RisuNest's own Rust crates and the patched upstream crates that `[patch.crates-io]` substitutes.
- `server/` holds RisuNest Sync: the Rust server and manager, plus two independent frontends (`server/manager/gui`, `server/endpoint-registry`) with their own lockfiles, installed with `pnpm --ignore-workspace`.
- `tests/` and `benchmarks/` hold verification harnesses that consume product code; `tests/README.md` describes the product/verification boundary.

## Building and Testing

- Build and run commands are in `README.md` and `package.json`. Release builds pass `--locked` to cargo, so update `Cargo.lock` before building.
- The repository has no formatter. Match the style of the surrounding file, and run `pnpm check` and the tests relevant to the changed area before committing or submitting a pull request.
- `pnpm test:full:shared` and `pnpm test:full` are the complete gates. They fail on missing prerequisites and never claim verification of another OS. `pnpm test:related` is local feedback only, not acceptance evidence.
- Platform parity is not a per-task test matrix. A shared-path change is verified by `pnpm check` and the tests for the changed area; build or run a platform only when the change reaches platform-conditional code (`isTauri`, `isMobile`, `isAndroid`, `isIOS`), `src-tauri` Rust, or the Android/iOS project files, and then only the platforms that code serves. Never run all five targets for one task.
- Node compatibility contracts compare against pinned RisuAI and PocketRisu checkouts; `.github/actions/test-references` and `tests/compatibleExportREADME.md` give the commits and the override variables. Never start or install the reference applications.
- Unit tests must not make unplanned network requests: the fetch and happy-dom guards fail teardown even for caught errors. A test that owns a loopback server allows one exact origin with `allowTestLoopbackOrigin` from `tests/support/testNetwork.ts` and revokes it in teardown.
- Source-only assertions read product files through `node:fs`, not `?raw` imports, which can hide uncovered Svelte lines from V8 coverage.
- `package.json` keeps the upstream RisuAI scripts first, then RisuNest additions grouped by area. Name a new script `<area>:<action>[:<variant>]` and hyphenate multi-word segments. `tauribuild*` are the Vite steps that `beforeBuildCommand` runs; a harness keeps its own Vite build inline in its Tauri config or runner instead of adding a script.

## Conventions and Architecture

- Svelte 5 runes (`$state`, `$derived`, `$effect`); shared stores live in `src/ts/stores.svelte.ts`. Use `.svelte.ts` for rune modules, `.svelte` for components, and camelCase file names.
- Use the custom theme colors from `src/styles.css` (`textcolor`, `textcolor2`, `bgcolor`, `darkbg`, `darkbutton`, `selected`, `borderc`, `darkborderc`, `draculared`; Tailwind opacity modifiers such as `text-textcolor/90` are fine). Consult `src/ts/gui/colorscheme.ts` only when working on theme logic.
- The revisioned persistent data store (`src/ts/storage/persistentDataStore.ts`) is authoritative; `DBState` is the in-memory compatibility working copy for existing UI and plugin paths. Blob and asset storage sits behind the BlobStore interface. Sync implementations stay separate adapters under `src/ts/storage/sync/`; the manifest-delta adapter has no server, do not extend it. `RisuSave` is the import/export/migration format, not the live store; imports are staged and validated before activation.
- Plugin system (API v3.0 only): iframe sandboxing, SafeDocument/SafeElement wrappers, save-specific and device-specific storage, custom AI providers, hot reload, and scalable API v3 queries. API v2.1 bundles cannot be installed and stored v2.1 records are skipped at load. See `plugins.md` and `src/ts/plugins/migrationGuide.md`.
- UI: no router, conditional rendering in `App.svelte`; the `theme` setting selects the GUI layout; in-app drag-and-drop uses the custom MIME types in `src/ts/dragTypes.ts`.
