# How To Release

A release is the tag `vX.Y.Z` on a commit of `main` whose workspace version is `X.Y.Z` and whose `ReleaseNotes.md` has
the section `## vX.Y.Z`. The tag starts the release workflow (`.github/workflows/release.yml`):

1. It checks the tag against the version and prints the notes section; a mismatch or a missing section fails the
   workflow before anything is built.
2. On Linux x64 and arm64 (Oracle Linux 8 containers, so the executables need glibc 2.28 or newer), Windows x64 and
   macOS arm64 and x64, it builds `Tools` and the samples in release, packs the Tools and Samples archives, unpacks them
   and runs `Tools` and a sample from the unpacked folders.
3. It publishes the GitHub release: the text is the notes section, the assets are the ten archives and `SHA256SUMS`.
4. It publishes the crates `dxfeed-graal-sys`, `dxfeed-graal-build` and `dxfeed-graal` on crates.io (see
   [Crates](#crates)). Pushing the tag is the decision to publish: nothing waits for an approval.

The items of a notes section start with the keys of their tickets (`**\[MDAPI-N]\[RUST]**`), as the titles of the pull
requests and their branches do (`[MDAPI-N][RUST] ...`, `feature/MDAPI-N-...`): the release resolves those tickets and
mails its notes.

## Versions

The version follows the Stability section of the README; from 1.0.0 on:

- **major** for a change that breaks code building against the public API outside the feature `experimental`:
  a removed or renamed item, a changed signature, a new required trait bound, a lost `Send` or `Sync`, a field or
  variant added to a type that is not `#[non_exhaustive]`;
- **minor** for new API, new fields and variants of `#[non_exhaustive]` types, any change behind `experimental`, a
  move to a newer SDK, and a higher minimum supported Rust version (say so in the notes);
- **patch** for fixes that change no API.

Before 1.0.0 a breaking change takes the next minor version. `dxfeed-graal-build` and, until 1.0.0, `dxfeed-graal-sys`
take the version of the workspace (`xtask release-notes` checks the latter); from 1.0.0 the bindings get a version of
their own, as the README describes.

## Prepare

On a branch, in a pull request:

1. Set the version in `Cargo.toml` (`[workspace.package] version`), in `dxfeed-graal-sys/Cargo.toml`, which keeps
   the SDK version as build metadata (`X.Y.Z+sdk.<SDK version>`), and in the `dxfeed-graal-sys` dependency of
   `dxfeed-graal/Cargo.toml`; the crate versions follow SemVer of the Rust API, not that of the SDK. Run `cargo check`
   to update `Cargo.lock`.
2. Add the section `## vX.Y.Z` to the top of `ReleaseNotes.md`.
3. Check the tag, the version of `dxfeed-graal-sys` and the notes locally:

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
failed jobs; the publish job runs once all the archives are built, and the `crates.io` job right after it (see
[Crates](#crates)). If a job fails before the crates are published and the code has to change, delete the release and
the tag (`git push origin :refs/tags/vX.Y.Z`, `git tag -d vX.Y.Z`), fix it in a pull request and tag again; once a
crate is on crates.io, its version is final, so fix it in the next version. A draft tag (below) tries the whole
release without publishing the crates.

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

## Crates

`dxfeed-graal-sys`, `dxfeed-graal-build` and `dxfeed-graal` go to crates.io; the tools, the samples and xtask are
`publish = false`. CI packs the three crates and builds them from the packages on every pull request
(`cargo package`), so a release commit is known to package.

After the GitHub release, the `crates.io` job of the release workflow publishes them, in the order of their
dependencies (`dxfeed-graal-sys`, `dxfeed-graal-build`, `dxfeed-graal`). It runs in the `crates-io` environment
(Settings, Environments), which:

- holds the crates.io API token as the secret `CARGO_REGISTRY_TOKEN`: the scopes `publish-new` (until the crates
  exist) and `publish-update`, limited to `dxfeed-graal*`, with an expiry; renew it there when it expires;
- lets only the tags `v*` in.

The environment has no required reviewer: the job publishes as soon as the GitHub release is out.

A draft tag publishes nothing on crates.io. A version that is already there is skipped, so a run that failed halfway
(the network, an expired token) can be re-run after the cause is fixed.

By hand, with a token of the owner, from the clean tag and Cargo 1.90 or newer (`--workspace` publishes in the same
order and skips the crates with `publish = false`):

```bash
git switch --detach vX.Y.Z
```

```bash
cargo publish --dry-run --workspace --locked
```

```bash
cargo publish --workspace --locked
```

If that stops halfway, publish the crates that are not on crates.io yet one by one, in the same order:
`cargo publish --locked -p <crate>`. Then check the pages of the crates on crates.io and their documentation on
docs.rs, which builds without the SDK (`DOCS_RS`) for the five supported targets (`[package.metadata.docs.rs]`).

A published version cannot be changed or removed, only yanked (`cargo yank --version X.Y.Z <crate>`), and crates.io
treats versions that differ only in the build metadata as one version: a new SDK needs a new version of
`dxfeed-graal-sys`, not only another `+sdk.` suffix.
