# dxFeed Graal Rust API

Rust API for [dxFeed](https://dxfeed.com) market data over the
[dxFeed Graal Native SDK](https://github.com/dxFeed/dxfeed-graal-native-sdk) C API (`dxfg_v4_` functions).

| Crate | Purpose |
|---|---|
| `dxfeed-graal-sys` | Raw FFI bindings to SDK 3.12.0; the build script gets and links the SDK |
| `dxfeed-graal` | Safe API: endpoints, feeds, subscriptions, listeners, events |

## Building

Supported targets: `x86_64-pc-windows-msvc`, `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`,
`x86_64-apple-darwin` and `aarch64-apple-darwin`. Rust 1.88 or newer.

The build script of `dxfeed-graal-sys` gets SDK 3.12.0 in one of these ways:

- `DXFEED_GRAAL_SDK_DIR` names an unpacked SDK archive (`graal-native-sdk-3.12.0-<platform>.zip` from the
  [release](https://github.com/dxFeed/dxfeed-graal-native-sdk/releases/tag/v3.12.0)). Use it for offline builds.
- Otherwise the default feature `download` fetches that archive from GitHub and checks its pinned SHA-256. The download
  honours the `ALL_PROXY`, `HTTPS_PROXY`, `HTTP_PROXY` and `NO_PROXY` environment variables and the operating system's certificate store.
- With `--no-default-features` and no `DXFEED_GRAAL_SDK_DIR`, the build fails.

The build fails if the SDK headers differ from those of 3.12.0, because the committed bindings fit only that version.
On docs.rs (`DOCS_RS` set) nothing is downloaded or linked.

## Running

`cargo run` and `cargo test` find the SDK library on their own. A binary started outside cargo needs the library
(`DxFeedGraalNativeSdk.dll`, `libDxFeedGraalNativeSdk.so` or `libDxFeedGraalNativeSdk.dylib`) next to the executable
on Windows, or on the loader path (`LD_LIBRARY_PATH`, `DYLD_LIBRARY_PATH`) or an rpath on Linux and macOS. A crate that
depends directly on `dxfeed-graal-sys` can read the library directory from `DEP_DXFEEDGRAALNATIVESDK_LIB_DIR` in its
build script.

## License

MPL-2.0, see [LICENSE](LICENSE).
