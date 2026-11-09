# Releasing Fastty

A release starts when a pull request merges into `main`. The automation reads
the version from `Cargo.toml`, creates the matching tag, and starts the build.
You do not push tags by hand.

## Publish a version

1. Set the new version in `Cargo.toml` and refresh the lock file:

   ```bash
   cargo set-version 0.25.5
   ```

2. Add the release notes to `CHANGELOG.md`. Use the heading
   `## 0.25.5 - YYYY-MM-DD`. The release body comes from this section. Without
   the section, the release publishes with no notes.

3. Commit the bump on the branch that will merge into `main`:

   ```bash
   git commit -m "chore(release): bump version to 0.25.5"
   ```

4. Open the pull request into `main` and merge it.

## What the automation does

1. `.github/workflows/release-on-merge.yml` runs when a pull request into `main`
   closes with a merge.
2. It reads `version` from `Cargo.toml` at the merge commit and builds the tag
   `vX.Y.Z`.
3. If the tag does not exist, it creates the tag on the merge commit and pushes
   it.
4. It starts `.github/workflows/release.yml` on the tag with `gh workflow run`.
5. `release.yml` builds Linux, macOS Intel, macOS ARM, and Windows. Then it
   publishes the GitHub Release with every asset, including `SHA256SUMS`.
6. The same workflow opens a pull request that updates `Casks/fastty.rb`.

Linux se construye en `ubuntu-22.04`, no en `ubuntu-latest`. El runner
`-latest` hoy es Ubuntu 26.04 (glibc 2.43) y deja el `.deb` y el AppImage sin
arrancar en Ubuntu 24.04 LTS y Debian 12. Si hay que moverlo, revisar
`dpkg-deb -I dist/*.deb` y confirmar que `libc6` no suba de 2.35.

The explicit start in step 4 is necessary. A tag pushed by a workflow with the
default `GITHUB_TOKEN` does not trigger other workflows. GitHub blocks events
created by the token to stop recursive runs.

## Rules

- The tag comes from `Cargo.toml`. A merge without a version bump creates no tag
  and no release. The job finishes green in about 10 seconds.
- Do not push the tag by hand. The workflow skips the tag when it already
  exists.
- If the tag exists but the release does not, the next merge with the same
  version publishes it.
- One build runs per tag. The `concurrency` group stops duplicate builds.

## CI

`.github/workflows/ci.yml` compiles `-p fastty` on macOS, Linux, and Windows on
every pull request, and runs the test suite on Linux. Before it existed, the
first place Linux and Windows were compiled was the release: v0.28.0 and v0.28.1
both failed on a single target and the whole release was skipped.

## Signing and notarization

### macOS

The release always signs. Without `FASTTY_SIGNING_IDENTITY` it signs ad-hoc,
which makes Gatekeeper ask the user to accept the app. The self-signed
certificate from `tools/setup-signing-cert.sh` keeps the designated requirement
stable across updates, but it cannot be notarized: notarization needs an Apple
**Developer ID Application** certificate.

Set these secrets to sign with Developer ID and notarize the DMG:

| Secret | Value |
| --- | --- |
| `FASTTY_SIGNING_IDENTITY` | `Developer ID Application: Name (TEAMID)` |
| `MACOS_NOTARY_APPLE_ID` | Apple ID |
| `MACOS_NOTARY_TEAM_ID` | Team ID |
| `MACOS_NOTARY_PASSWORD` | app-specific password |

With those present, `release.yml` signs the app with `--options runtime
--timestamp`, signs the DMG, submits it to `notarytool`, staples the ticket, and
checks it with `spctl`. Without them the DMG is published ad-hoc and users need
`xattr -cr /Applications/Fastty.app` once.

### Windows

Without a certificate, `fastty.exe` and the MSI are unsigned and SmartScreen
shows "Unknown publisher". To sign, add:

| Secret | Value |
| --- | --- |
| `WINDOWS_CERTIFICATE_BASE64` | `base64 -i cert.pfx` |
| `WINDOWS_CERTIFICATE_PASSWORD` | password of the `.pfx` |

The workflow signs the executable before zipping it and the MSI after building
it.

## Checksums

The release job writes `SHA256SUMS` next to the assets. `src/updater.rs` looks
for `<asset>.sha256`, `SHA256SUMS`, or `checksums.txt` and verifies the
download when one exists; without the file it skipped verification.

## Recover a release

Start the build for a tag that already exists:

```bash
gh workflow run release.yml --ref v0.25.5
```

You can also run the `Release` workflow from the Actions tab and pass the `tag`
input. This path works when the workflow runs on a branch.

## Check the result

```bash
gh run list --limit 5
gh release view v0.25.5
```
