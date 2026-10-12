# Releasing RisuNest

Stable releases are published automatically when a matching tag is pushed:

- App: `app-v<version>`
- Sync: `sync-v<version>`

Use a stable SemVer version and tag a reviewed commit contained in `main`.

## Prepare the release commit

For an App release, set the same version in:

- `version.json`
- `src-tauri/tauri.conf.json`
- `release-notes/app/<version>.md`

For a Sync release, set the same version in:

- `server/version.json`
- `server/sync/Cargo.toml`
- `server/manager/Cargo.toml`
- `server/manager/gui/package.json`
- `server/manager/gui/src-tauri/Cargo.toml`
- `server/manager/gui/src-tauri/tauri.conf.json`
- `release-notes/sync/<version>.md`

Update every affected lockfile in the same commit. Release notes must begin with a
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
3. Run a non-publishing rehearsal of the exact reviewed commit. The workflow
   rejects a `source_ref` that is not contained in `origin/main`, and a tag that
   already exists on another commit:

   ```powershell
   $Sha = (git rev-parse HEAD).Trim()
   gh workflow run release.yml --ref main `
       -f product=app -f tag=app-v1.0.0 -f source_ref=$Sha `
       -f publish=false -f bootstrap=false
   gh workflow run release.yml --ref main `
       -f product=sync -f tag=sync-v1.0.0 -f source_ref=$Sha `
       -f publish=false -f bootstrap=false
   ```

   `--ref` selects the workflow definition; `source_ref` selects the commit to
   build. The tag may be unpushed for a rehearsal. A rehearsal still uses the
   production signing Secrets.
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
