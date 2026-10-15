# Releasing RisuNest

Stable releases are published automatically when a matching tag is pushed:

- App: `app-v<version>`
- Sync: `sync-v<version>`

Use a stable SemVer version and tag a reviewed commit contained in `main`.

## Prepare the release commit

For an App release, set the same version in:

- `package.json`
- `version.json`
- `src-tauri/Cargo.toml`
- `src-tauri/tauri.conf.json`
- `src-tauri/gen/apple/project.yml` (`CFBundleShortVersionString` and `CFBundleVersion`)
- `src-tauri/gen/apple/risunest_iOS/Info.plist` (both bundle version fields)
- `release-notes/app/<version>.md`

For a Sync release, set the same version in:

- `server/version.json`
- `server/sync/Cargo.toml`
- `server/manager/Cargo.toml`
- `server/manager/gui/package.json`
- `server/manager/gui/src-tauri/Cargo.toml`
- `server/manager/gui/src-tauri/tauri.conf.json`
- `release-notes/sync/<version>.md`

Update every affected lockfile in the same commit, including the App dependency
entries in `tests/native/native/Cargo.lock`, `benchmarks/ios/native/Cargo.lock`,
and `benchmarks/macos/native/Cargo.lock`. Release notes must begin with a
Markdown title and contain non-empty `## English` and `## 한국어` sections.

## Repository configuration

The release workflows use these repository Actions Secrets:

- `TAURI_PRIVATE_KEY`
- `TAURI_KEY_PASSWORD` (optional for an unencrypted update key)
- `RISUNEST_UPDATE_PUBLIC_KEY`
- `ANDROID_KEYSTORE_BASE64` (App only)
- `ANDROID_KEYSTORE_PASSWORD` (App only)
- `ANDROID_KEY_ALIAS` (App only)
- `ANDROID_KEY_PASSWORD` (App only)
- `RISUNEST_DEFAULT_REGISTRY_URL` (Sync only)

Keep the update signing identity and Android signing identity stable across
releases. Never commit private keys, passwords, or keystores.

`RISUNEST_RELEASE_BOOTSTRAP` is a repository Actions Variable. Keep it `false`
except when publishing the first product into an empty release catalogue. Set it
back to `false` immediately after that first publication succeeds.

## Rehearse without preparing a new release

Run `Release build` with `publish=false` and leave `tag` empty. The workflow
uses the selected source's App or Sync version and existing release notes.
Version files must agree and the notes must pass the same validation as a
publication. No version bump, new release notes, or Git tag is needed.

```powershell
$Sha = (git rev-parse HEAD).Trim()
gh workflow run release.yml --ref main `
    -f product=app -f source_ref=$Sha -f publish=false -f bootstrap=false
gh workflow run release.yml --ref main `
    -f product=sync -f source_ref=$Sha -f publish=false -f bootstrap=false
```

`--ref` selects the workflow definition; `source_ref` selects the source to
build and must be contained in `origin/main`. Use a full commit SHA to keep
retries on the same source. Dispatch a new run to verify another commit or
repeat the whole build after earlier artifacts expire.

A rehearsal may reuse a version whose tag already points to an older commit.
An explicit `tag`, if provided, must still match the selected product version.
Builds use the production signing Secrets and run the package verification
gates, but do not create or modify GitHub Releases, tags, the update catalog,
or README downloads. Download packages from that run's `rehearsal-*` artifacts;
the release URLs in its validation manifest are not rehearsal download links.
Use the run ID and recorded source commit to distinguish builds of the same
version. Rehearsal packages and manifests are retained for two days.

Before publishing a new version, prepare its release commit and rehearse that
exact commit again. Same-version rehearsals do not prove an upgrade from the
latest published version to the next version; the Sync Windows installer checks
start from the newest published Sync version lower than the source version.

## Before the first publication

Pushing an `app-v*` or `sync-v*` tag publishes a release. Never push a tag to
test the release build.

1. Review everything that becomes public. Changing the repository to public
   also exposes existing Actions runs, logs, and artifacts, so review those as
   well as every branch and tag that will be visible. A clean current snapshot
   does not make older history or uploaded logs safe.
2. Compare the configured signing identity with the intended one: the update
   public key in `RISUNEST_UPDATE_PUBLIC_KEY` and the SHA-256 fingerprint of the
   Android release certificate. The workflow's signing preflight proves that
   the Secrets form a working key pair and keystore entry; it does not prove
   that they are the intended identity.
3. Run a non-publishing rehearsal of the exact reviewed commit using the
   commands above. Compare the recorded source commit with the intended SHA.
4. Check the rehearsal's `rehearsal-*` artifacts: every expected package and
   signature is present, versions and the source commit match, and nothing
   unexpected is included.

Publish only after the rehearsal of that same commit succeeds.

## Publish

Create the tag from the prepared release commit and push that exact tag:

```powershell
$Sha = git rev-parse HEAD
git tag app-v1.0.0 $Sha
git push origin refs/tags/app-v1.0.0
```

For Sync, use the corresponding Sync tag:

```powershell
$Sha = git rev-parse HEAD
git tag sync-v1.0.0 $Sha
git push origin refs/tags/sync-v1.0.0
```

The tag version must match the product version files and release-notes filename.
If both products are released from the same commit, create both tags on that
commit and push both explicit tag refs together.

## Update your local branch

After publication, the workflow updates the download sections in `README.md` and
`README.ko.md` and commits any changes to `main`. With a clean working tree and no
unpushed commits, run `git pull --ff-only origin main` from your local `main`.
If you have local-only commits, rebase them onto the updated remote branch before
pushing:

```bash
git pull --rebase origin main
git push origin main
```

Resolve any conflicts before continuing the rebase and pushing.

## Published releases are immutable

Do not move or reuse a published tag. Do not replace assets on a published
release. Publish a new version instead.
