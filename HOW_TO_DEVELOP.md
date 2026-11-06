# How To Develop

Regenerate the bindings after an SDK update (needs libclang; set `LIBCLANG_PATH` if another one is found first):

```bash
cargo run -p xtask -- gen-bindings <unpacked SDK dir>
```

Check that `samples/deploy` runs outside cargo with only the deployed library next to it, fails without it, and that a
rebuild does not copy the library again:

```bash
cargo run -p xtask -- check-deploy
```

The tests expect no `DXFEED_*` environment variables: the SDK reads them as system properties in every test process.

The cargo feature `experimental` adds items, so run clippy, rustdoc and the tests with and without `--all-features`.
