# dxfeed-graal-sys

Raw FFI bindings to the C API (`dxfg_v4_` functions) of the
[dxFeed Graal Native SDK](https://github.com/dxFeed/dxfeed-graal-native-sdk) 3.16.0. Use the safe API of the
[`dxfeed-graal`](https://crates.io/crates/dxfeed-graal) crate instead, unless you need the raw functions.

The bindings are generated and committed, so the build needs neither libclang nor the SDK headers. The build script
gets the SDK library for the target:

- from `DXFEED_GRAAL_SDK_DIR`, an unpacked SDK archive (`graal-native-sdk-<version>-<platform>.zip`), for offline builds;
- otherwise (the default feature `download`) from the GitHub release of the SDK, verified against a pinned SHA-256;
  `DXFEED_GRAAL_SDK_CACHE_DIR`, an absolute directory, keeps the archive across builds.

Supported targets: `x86_64-pc-windows-msvc`, `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`,
`x86_64-apple-darwin` and `aarch64-apple-darwin`. See the
[repository](https://github.com/dxFeed/dxfeed-graal-rust-api) for the requirements of each platform.

The crate version is that of the Rust API; the build metadata (`+sdk.<version>`) names the SDK.

License: [MPL-2.0](https://github.com/dxFeed/dxfeed-graal-rust-api/blob/main/LICENSE).
