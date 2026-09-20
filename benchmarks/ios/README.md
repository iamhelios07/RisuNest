# iOS native verification

Builds and runs isolated iOS product and harness targets with synthetic fixtures. It uses a separate application identifier and never reads installed app data.

The build-lab `ios-check.yml` workflow runs simulator storage, restart, streaming, file, lifecycle, product UI, and XCTest checks from an audited source snapshot.

## Physical-device package

The manual `ios-testlab-package.yml` workflow produces:

- `RisuNest-ios-testlab.zip`, containing the app, UI test runner, and `.xctestrun`
- `RisuNest-ios-device-agent.ipa`, an agent build that is not a distribution build
- `testlab-package.json`, containing source and toolchain provenance plus hashes

Inspect the current Firebase device and Xcode catalog before choosing IDs:

```sh
gcloud firebase test ios models list --project=YOUR_PROJECT
gcloud firebase test ios versions list --project=YOUR_PROJECT
```

Run an authorized test with verified values:

```sh
gcloud firebase test ios run --project=YOUR_PROJECT --type=xctest \
  --test=RisuNest-ios-testlab.zip --timeout=15m \
  --device=model=VERIFIED_MODEL,version=VERIFIED_OS,locale=en_US,orientation=portrait \
  --xcode-version=VERIFIED_XCODE
```

Firebase execution is never part of the packaging workflow and requires separate authorization. Package generation does not establish a device pass. Long locked-device, battery, thermal, iCloud provider, camera-content, and distribution-signing checks remain separate.

See `docs/ios-testing.md` for the current build-lab and device procedure.
