# Product and verification boundaries

`pnpm exec vitest run --project app-lua-worker` reruns the core scripting and history
contracts with actual Worker message transport and the shipped Lua WASM. `pnpm test`
includes this project. Cases that inject a main-thread clock or inspect its factory
run in the ordinary project; Worker deadline and recovery cases run in
`src/ts/process/scriptings.worker.test.ts`.

`support/responsesInternals.ts` is installed only by `vitest.config.ts`. It lets
the existing exact Responses assertions inspect private functions in Vitest's
transformed module without changing provider algorithms or product exports.

Check the bundle guard itself:

```sh
node --test tests/productionBundle.node-test.mjs
```

Build and inspect normal application outputs (including dynamic chunks, emitted
worker JS, and source maps) without overwriting `dist`:

```sh
pnpm check:production-bundle desktop
pnpm check:production-bundle android
pnpm check:production-bundle production
```

The check deliberately sets the two removed harness flags to prove that they
cannot enable verification code in a product build. Public assets are not copied.

Standalone tokenizer and streaming harnesses live under `benchmarks/` and consume
product code. `pnpm check:benchmark-harnesses` checks their TypeScript and Svelte.
Their build commands and platform constraints are in the respective READMEs.

Android APK alignment regression fixtures require PowerShell 7, Android SDK
build-tools 36.0.0 and NDK 28.2.13676358. Run them separately from the portable
JavaScript suites:

```powershell
pwsh -NoProfile -File tests/androidAlignment.ps1 -SdkRoot $env:ANDROID_HOME -NdkRoot $env:NDK_HOME
```

The runner creates synthetic ELF/ZIP fixtures and invokes the actual alignment
checker and Android tools. It checks case-sensitive resource names, missing and
duplicate native libraries, and invalid ELF LOAD alignment. The PowerShell
runner is outside the JavaScript test inventory and must be invoked explicitly.
