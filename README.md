# dxFeed Graal Rust API

Rust API for [dxFeed](https://dxfeed.com) market data over the
[dxFeed Graal Native SDK](https://github.com/dxFeed/dxfeed-graal-native-sdk) C API (`dxfg_v4_` functions).

| Crate | Purpose |
|---|---|
| `dxfeed-graal-sys` | Raw FFI bindings to SDK 3.12.0; the build script gets and links the SDK |
| `dxfeed-graal` | Safe API: endpoints, feeds, subscriptions, listeners, events |

## License

MPL-2.0, see [LICENSE](LICENSE).
