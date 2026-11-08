# dxfeed-graal-build

A build-script helper for applications that use [`dxfeed-graal`](https://crates.io/crates/dxfeed-graal). The
dxFeed Graal Native SDK is a dynamic library: `cargo run` and `cargo test` find it, but an executable started
outside cargo needs the library next to it. `deploy()` copies the library into the cargo profile directory, where the
executables are written, and makes Linux and macOS executables look for it in their own directory.

```toml
[dependencies]
dxfeed-graal = "0.1"

[build-dependencies]
dxfeed-graal-build = "0.1"
```

```rust
// build.rs
fn main() {
    dxfeed_graal_build::deploy().unwrap();
}
```

Copy the executable together with the library to run it elsewhere. See the
[documentation](https://docs.rs/dxfeed-graal-build) for the details and the limits.

License: [MPL-2.0](https://github.com/dxFeed/dxfeed-graal-rust-api/blob/main/LICENSE).
