# iOS native verification

Builds isolated iOS test targets with synthetic fixtures and a separate application identifier.

## Physical-device package

Use macOS with Xcode, XcodeGen, Node.js, pnpm, Python 3, and Rust's `aarch64-apple-ios` target installed. Install the repository's dependencies with `pnpm install --frozen-lockfile` and initialize the product's Apple project with `pnpm exec tauri ios init --ci`. Run from the repository root:

```sh
TAURI_ENV_PLATFORM=ios python3 benchmarks/ios/buildTestLab.py
```

The command builds and checks arm64 iPhoneOS artifacts with a minimum iOS version of 16.4. It writes these files under `artifacts/`:

- `RisuNest-ios-testlab.zip`, containing the app, UI test runner, and `.xctestrun`
- `RisuNest-ios-device-agent.ipa`, an agent build that is not a distribution build
- `testlab-package.json`, containing the Xcode version, target, signing type, file sizes, and SHA-256 hashes

Use a fresh `artifacts/testlab-build` directory for each run. The packages use ad-hoc signing and are not distribution builds. Package generation does not run a device test or contact a remote test service.
