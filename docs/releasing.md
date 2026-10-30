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
   publishes the GitHub Release with every asset.
6. The same workflow opens a pull request that updates `Casks/fastty.rb`.

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
