# dxFeed Graal Rust API

This package provides access to [dxFeed market data](https://dxfeed.com/market-data/).
The library is built as a language-specific wrapper over
the [dxFeed Graal Native](https://github.com/dxFeed/dxfeed-graal-native-sdk) library,
which was compiled with [GraalVM Native Image](https://www.graalvm.org/latest/reference-manual/native-image/)
and [dxFeed Java API](https://docs.dxfeed.com/dxfeed/api/overview-summary.html) (our flagman API).

> [!WARNING]
> The project is in **alpha**: the API may change in any release, and only a part of the SDK is covered (see
> [Current State](#current-state)).

![Status](https://img.shields.io/badge/status-alpha-red)
![Graal Native SDK](https://img.shields.io/badge/Graal%20Native%20SDK-v3.12.0-yellow)
![Platform](https://img.shields.io/badge/platform-win--x64%20%7C%20linux--x64%20%7C%20linux--aarch64%20%7C%20osx--x64%20%7C%20osx--aarch64-lightgrey)
![Rust](https://img.shields.io/badge/Rust-1.88%2B-blueviolet)
[![License](https://img.shields.io/badge/license-MPL--2.0-orange)](LICENSE)

## Table of Contents

- [Overview](#overview)
  * [Implementation Details](#implementation-details)
- [Documentation](#documentation)
- [Requirements](#requirements)
- [Installation](#installation)
- [Building](#building)
- [Running and Deployment](#running-and-deployment)
- [Usage](#usage)
  * [Threads and Listeners](#threads-and-listeners)
  * [Closing and Exit](#closing-and-exit)
- [Samples](#samples)
- [Current State](#current-state)
- [Development](#development)
- [License](#license)

## Overview

dxFeed Graal Rust API lets Rust applications receive and publish dxFeed market data with a safe, idiomatic API:
owned handles released on drop, `Result` errors that carry Java exceptions, and closures as event listeners.
The names follow the [dxFeed Java API](https://docs.dxfeed.com/dxfeed/api/overview-summary.html): `DXEndpoint`,
`DXFeed`, `DXFeedSubscription` and `DXPublisher`, with methods in snake case (`add_event_listener`,
`close_and_await_termination`) and getters without `get` (`feed()`, `publisher()`). Events print as Java prints them.

| Crate | Purpose |
|---|---|
| `dxfeed-graal-sys` | Raw FFI bindings to SDK 3.12.0; the build script gets and links the SDK |
| `dxfeed-graal` | Safe API: endpoints, feeds, subscriptions, listeners, events, publishing |
| `dxfeed-graal-build` | Build-script helper that deploys the SDK library next to the application's executables |

### Implementation Details

We use [GraalVM Native Image](https://www.graalvm.org/latest/reference-manual/native-image/) technology and specially
written code that *wraps* Java methods into native ones
to get dynamically linked libraries for different platforms (Linux, macOS, and Windows) based on
the [latest Java API package](https://dxfeed.jfrog.io/artifactory/maven-open/com/devexperts/qd/dxfeed-api/).

Then, the resulting dynamic link library (dxFeed Graal-native) is used through
C [ABI](https://en.wikipedia.org/wiki/Application_binary_interface) (application binary interface), its `dxfg_v4_`
functions, and we write programming interfaces that describe our business model (similar to Java API).

## Documentation

- API documentation: `cargo doc --open -p dxfeed-graal`
- [dxFeed Knowledge Base](https://kb.dxfeed.com/index.html?lang=en)
  * [Getting Started](https://kb.dxfeed.com/en/getting-started.html)
  * [Troubleshooting](https://kb.dxfeed.com/en/troubleshooting-guidelines.html)
  * [Market Events](https://kb.dxfeed.com/en/data-model/dxfeed-api-market-events.html)
  * [Event Delivery contracts](https://kb.dxfeed.com/en/data-model/model-of-event-publishing.html#event-delivery-contracts)
  * [Exchange Codes](https://kb.dxfeed.com/en/data-model/exchange-codes.html)
  * [Symbology Guide](https://kb.dxfeed.com/en/data-model/symbology-guide.html)

## Requirements

64-bit targets only: `x86_64-pc-windows-msvc`, `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`,
`x86_64-apple-darwin` and `aarch64-apple-darwin`. Rust 1.88 or newer.

### Windows

x64 only. The SDK library needs the
[Visual C++ Redistributable for Visual Studio 2015 or newer](https://aka.ms/vs/17/release/vc_redist.x64.exe)
(`VCRUNTIME140.dll`, `VCRUNTIME140_1.dll`).

### Linux

x64 and aarch64 with [glibc](https://www.gnu.org/software/libc/); musl is not supported. The SDK library needs
`libz.so.1` and `libpthread.so.0`.

### macOS

x64 and Arm64.

## Installation

The crates are not published on crates.io yet. Depend on them by path or by git:

```toml
[dependencies]
dxfeed-graal = { path = "../dxfeed-graal-rust-api/dxfeed-graal" }
```

## Building

The build script of `dxfeed-graal-sys` gets SDK 3.12.0 in one of these ways:

- `DXFEED_GRAAL_SDK_DIR` names an unpacked SDK archive (`graal-native-sdk-3.12.0-<platform>.zip` from the
  [release](https://github.com/dxFeed/dxfeed-graal-native-sdk/releases/tag/v3.12.0)). Use it for offline builds.
- Otherwise the default feature `download` fetches that archive from GitHub and checks its pinned SHA-256. The download
  honours the `ALL_PROXY`, `HTTPS_PROXY`, `HTTP_PROXY` and `NO_PROXY` environment variables and the operating system's
  certificate store.
- `DXFEED_GRAAL_SDK_CACHE_DIR`, an absolute path, keeps the downloaded archive in that directory, so a clean build or
  another target directory reuses it instead of downloading 21 MB again. The archive's SHA-256 is checked on every
  reuse; one that does not match is downloaded again with a warning. Without the variable nothing is written outside
  the build directory.
- With `--no-default-features` and no `DXFEED_GRAAL_SDK_DIR`, the build fails.

The build fails if the SDK headers differ from those of 3.12.0, because the committed bindings fit only that version.
On docs.rs (`DOCS_RS` set) nothing is downloaded or linked.

## Running and Deployment

`cargo run` and `cargo test` find the SDK library on their own. An executable started outside cargo needs the library
(`DxFeedGraalNativeSdk.dll`, `libDxFeedGraalNativeSdk.so` or `libDxFeedGraalNativeSdk.dylib`) next to it. The
`dxfeed-graal-build` crate puts it there from the application's build script:

```toml
[dependencies]
dxfeed-graal = { path = "../dxfeed-graal-rust-api/dxfeed-graal" }

[build-dependencies]
dxfeed-graal-build = { path = "../dxfeed-graal-rust-api/dxfeed-graal-build" }
```

```rust,ignore
// build.rs
fn main() {
    dxfeed_graal_build::deploy().unwrap();
}
```

`deploy()` copies the library into the profile directory (`target/debug`, `target/release`, or
`target/<triple>/<profile>` when cross-compiling), where cargo puts the executables, and skips the copy when the
library has not changed. On Linux and macOS it also sets the executables' rpath to their own directory
(`$ORIGIN`, `@loader_path`), so they find the library without `LD_LIBRARY_PATH`. To ship the application, copy the
executable together with the library. Examples and tests are not covered; they run through cargo.

For another layout, or when `build.build-dir` moves build scripts out of the target directory, call
`dxfeed_graal_build::library_path()` and copy the library yourself. The macOS rpath is not verified yet.
[`samples/deploy`](samples/deploy) is a complete application, and `cargo run -p xtask -- check-deploy` checks that it
runs from a directory holding only the executable and the library.

## Usage

How to connect to a QD endpoint and print quotes until Enter is pressed:

```rust
use dxfeed_graal::DXEndpoint;
use dxfeed_graal::event::Quote;

fn main() -> dxfeed_graal::Result<()> {
    let endpoint = DXEndpoint::create()?;
    let subscription = endpoint.feed().create_subscription::<Quote>()?;
    subscription.add_event_listener(|quotes| {
        for quote in quotes {
            println!("{quote}");
        }
    })?;
    subscription.add_symbols(["AAPL"])?;
    endpoint.connect("demo.dxfeed.com:7300")?;
    std::io::stdin().read_line(&mut String::new()).ok();
    endpoint.close_and_await_termination()
}
```

A quote prints as in Java: `Quote{AAPL, eventTime=0, time=20241004-093000.000, timeNanoPart=0, sequence=0, ...}`.

- A subscription to one event type, `create_subscription::<Quote>()`, gives its listener `&QuoteRef` values; a
  subscription to several, `create_subscription_with_types(&[EventType::Quote, EventType::Trade])`, gives `EventRef`
  values to match.
- Calls chain as in Java: a feed or a subscription taken from a chain keeps its endpoint open, e.g.
  `let feed = DXEndpoint::create()?.connect("demo.dxfeed.com:7300")?.feed();`.
- `DXFeed::get_instance()` and `DXEndpoint::get_instance()` return the default feed and endpoint of the process,
  configured by the `dxfeed.properties` file and the system properties (`system::set_property("dxfeed.address", ...)`)
  on first use.

### Threads and Listeners

- All types are `Send` and `Sync`. Threads are attached to the SDK on their first call and detached when they exit.
- A listener's closure runs on a thread of the endpoint's executor, or inside `ManualExecutor::process_all_pending`.
  The calls of one listener do not overlap. The events are valid during the call only: copy what you keep with
  `to_owned()`. A panic in a closure is caught and printed to stderr; the listener stays active.
- Add listeners before symbols.
- Removing a listener or dropping its subscription waits for its running calls (inside a callback it does not), so do
  not hold a lock that a closure takes while doing that from another thread.

### Closing and Exit

- `DXEndpoint::close_and_await_termination()` closes an endpoint and waits for the notifications already scheduled;
  `close()` does not wait. Dropping the last handle of an endpoint (the endpoint, its feed, its publisher or a
  subscription) closes it as well.
- The process may exit with open endpoints and running listeners: from the moment it starts exiting, listeners are no
  longer called and destructors no longer call the SDK.
- `system::shutdown()` is an advanced function with no counterpart in the other dxFeed APIs. It stops all listeners,
  waiting for the running calls, and closes all endpoints of the process. Afterwards
  nothing new can be created (`Error::ShutDown`), while existing values can still be closed and dropped. It cannot be
  called from a listener (`Error::InCallback`). While it runs, an endpoint on a `ManualExecutor` needs another thread
  calling `process_all_pending`.
- Only executables are supported: a Rust `cdylib` loaded as a DLL is not.

## Samples

- [print_quotes](dxfeed-graal/examples/print_quotes.rs) prints the quotes of a symbol until Enter is pressed, like
  `PrintQuoteEventsSample` of the other dxFeed APIs:

  ```bash
  cargo run --example print_quotes -- [SYMBOL] [ADDRESS]
  ```

  The defaults are `AAPL` and `demo.dxfeed.com:7300`.
- [deploy](samples/deploy) is an application whose build script deploys the SDK library next to its executable (see
  [Running and Deployment](#running-and-deployment)).

## Current State

### Endpoint Roles

- [x] `Feed`, `OnDemandFeed`, `StreamFeed`
- [x] `Publisher`, `StreamPublisher`
- [x] `LocalHub`: events published via its `Publisher` are delivered to its own `Feed` only

### Event Types

- [x] [Quote](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/market/Quote.html)
- [x] [Trade](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/market/Trade.html)
- [x] [TradeETH](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/market/TradeETH.html)
- [x] [Profile](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/market/Profile.html)
- [x] [Summary](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/market/Summary.html)
- [x] [Greeks](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/option/Greeks.html)
- [x] [TheoPrice](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/option/TheoPrice.html)
- [x] [Underlying](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/option/Underlying.html)
- [x] [Series](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/option/Series.html)
- [x] [NuamTrade](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/custom/NuamTrade.html)
- [ ] Other event types arrive as `EventRef::Other`

### Features

- [x] System properties, the default endpoint and feed (`get_instance`)
- [x] Typed and untyped subscriptions to string symbols, listeners, publishing
- [x] Events printed in the format of Java's `toString`
- [x] `close_and_await_termination`, explicit shutdown and clean process exit
- [x] Deployment of the SDK library next to executables, a download cache for the SDK archive
- [ ] Time series subscriptions, models, IPF, schedules, other symbol types

## Development

Regenerate the bindings after an SDK update (needs libclang; set `LIBCLANG_PATH` if another one is found first):

```bash
cargo run -p xtask -- gen-bindings <unpacked SDK dir>
```

Check that `samples/deploy` runs outside cargo with only the deployed library next to it, fails without it, and that a
rebuild does not copy the library again:

```bash
cargo run -p xtask -- check-deploy
```

## License

MPL-2.0, see [LICENSE](LICENSE).
