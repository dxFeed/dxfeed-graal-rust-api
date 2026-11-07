# How To Develop

Regenerate the bindings after an SDK update (needs libclang; set `LIBCLANG_PATH` if another one is found first):

```bash
cargo run -p xtask -- gen-bindings <unpacked SDK dir>
```

Check that a sample (`WriteTapeFileSample`) runs outside cargo with only the deployed library next to it, fails without
it, and that a rebuild does not copy the library again:

```bash
cargo run -p xtask -- check-deploy
```

The tests expect no `DXFEED_*` environment variables except the build ones (`DXFEED_GRAAL_SDK_*`): the SDK reads them
as system properties in every test process.

The cargo feature `experimental` adds items, so run clippy, rustdoc and the tests with and without `--all-features`.

CI (`.github/workflows/build.yml`) runs the same checks on a pull request and a push to main: fmt and rustdoc (also
without the SDK, as on docs.rs) on Linux; on Linux x64 and arm64, Windows x64 and macOS arm64 and x64, clippy and the
tests in debug and release, each with and without `--all-features`, and then `check-deploy`; and `cargo check` on the
minimum supported Rust version. It uses the toolchain set in the workflow (`RUSTUP_TOOLCHAIN`); update it there together with the code that
a newer clippy needs.

The release workflow (`.github/workflows/release.yml`, see [HOW_TO_RELEASE.md](HOW_TO_RELEASE.md)) also runs on a pull
request that touches it, the setup action, `xtask`, the tools, the samples, the deployment of the SDK library,
`ReleaseNotes.md` or the root manifest: it builds and checks the Tools and Samples archives of every platform and
publishes nothing. Its toolchain (`RUSTUP_TOOLCHAIN`) is the one of `build.yml`; change them together.
