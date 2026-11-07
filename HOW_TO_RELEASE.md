# How To Release

A release is the tag `vX.Y.Z` on a commit of `main` whose workspace version is `X.Y.Z` and whose `ReleaseNotes.md` has
the section `## vX.Y.Z`. The tag starts the release workflow (`.github/workflows/release.yml`):

1. It checks the tag against the version and prints the notes section; a mismatch or a missing section fails the
   workflow before anything is built.
2. On Linux x64 and arm64 (Oracle Linux 8 containers, so the executables need glibc 2.28 or newer), Windows x64 and
   macOS arm64 and x64, it builds `Tools` and the samples in release, packs the Tools and Samples archives, unpacks them
   and runs `Tools` and a sample from the unpacked folders.
3. It publishes the GitHub release: the text is the notes section, the assets are the ten archives and `SHA256SUMS`.

The crates are not published on crates.io.

## Prepare

On a branch, in a pull request:

1. Set the version in `Cargo.toml` (`[workspace.package] version`) and in `dxfeed-graal-sys/Cargo.toml`, which keeps
   the SDK version as build metadata (`X.Y.Z+sdk.3.13.0`); the crate versions follow SemVer of the Rust API, not that
   of the SDK. Run `cargo check` to update `Cargo.lock`.
2. Add the section `## vX.Y.Z` to the top of `ReleaseNotes.md`.
3. Check the tag and the notes locally:

   ```bash
   cargo run -p xtask -- release-tag
   ```

   ```bash
   cargo run -p xtask -- release-notes vX.Y.Z
   ```

The pull request runs the release workflow too (it touches `Cargo.toml` and `ReleaseNotes.md`): it builds and checks
the archives of every platform and publishes nothing. Merge it when both workflows are green.

## Release

Tag the merge commit on `main` and push the tag:

```bash
git switch main
```

```bash
git pull --ff-only
```

```bash
git tag vX.Y.Z
```

```bash
git push origin vX.Y.Z
```

Follow the run on the Actions page. If a job fails for a reason outside the code (a runner, the network), re-run the
failed jobs; the publish job runs once all the archives are built. If the code has to change, delete the tag
(`git push origin :refs/tags/vX.Y.Z`, `git tag -d vX.Y.Z`), fix it in a pull request and tag again.

## Prereleases and drafts

The tag decides how the release is published:

- `vX.Y.Z`: a release, marked as the latest one.
- `vX.Y.Z-<suffix>` (`v0.2.0-rc.1`): a prerelease; it needs the version `X.Y.Z-<suffix>` in `Cargo.toml` and the
  section `## vX.Y.Z-<suffix>`.
- A suffix with `draft` (`v0.2.0-draft.1`, with the same version and section): a draft prerelease, seen only by the
  maintainers; publish or delete it on the Releases page, and delete its tag with it.

A trial run that publishes nothing: start the release workflow by hand (Actions, Release, Run workflow) on any branch;
it builds and checks the archives of the workspace version and keeps them as artifacts of the run.

## Archives

`dxFeedGraalRustApi-<tag>-<arch>-<os>-Tools` and `-Samples`, where `<arch>` is `x86_64` or `aarch64` and `<os>` is
`linux`, `windows` or `osx`: `.zip` on Windows, `.tar.gz` elsewhere. Each one holds a folder of the same name with
`LICENSE` and `bin/`: the executables, the SDK library and, in the Samples archive, the `dxfeed.system.properties` of
the samples that read the default feed. Build and check them locally from a release build:

```bash
cargo build --release --locked -p dxfeed-graal-tools -p dxfeed-graal-samples
```

```bash
cargo run -p xtask -- package vX.Y.Z dist
```
