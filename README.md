<picture>
 <source media="(prefers-color-scheme: dark)" srcset="docs/images/logo_dark.svg">
 <img alt="dxFeed" src="docs/images/logo_light.svg">
</picture>

This package provides access to [dxFeed market data](https://dxfeed.com/market-data/).
The library is built as a language-specific wrapper over
the [dxFeed Graal Native](https://github.com/dxFeed/dxfeed-graal-native-sdk) library,
which was compiled with [GraalVM Native Image](https://www.graalvm.org/latest/reference-manual/native-image/)
and [dxFeed Java API](https://docs.dxfeed.com/dxfeed/api/overview-summary.html) (our flagman API).

> [!WARNING]
> The project is in **alpha**: the API may change in any release (see [Current State](#current-state)).

[![Build](https://github.com/dxFeed/dxfeed-graal-rust-api/actions/workflows/build.yml/badge.svg?branch=main)](https://github.com/dxFeed/dxfeed-graal-rust-api/actions/workflows/build.yml)
[![Release](https://img.shields.io/github/v/release/dxFeed/dxfeed-graal-rust-api?include_prereleases)](https://github.com/dxFeed/dxfeed-graal-rust-api/releases)
[![Crates.io](https://img.shields.io/crates/v/dxfeed-graal)](https://crates.io/crates/dxfeed-graal)
[![docs.rs](https://img.shields.io/docsrs/dxfeed-graal)](https://docs.rs/dxfeed-graal)
![Status](https://img.shields.io/badge/status-alpha-red)
![Graal Native SDK](https://img.shields.io/badge/Graal%20Native%20SDK-v3.15.0-yellow)
![Platform](https://img.shields.io/badge/platform-win--x64%20%7C%20linux--x64%20%7C%20linux--aarch64%20%7C%20osx--x64%20%7C%20osx--aarch64-lightgrey)
![Rust](https://img.shields.io/badge/Rust-1.88%2B-blueviolet)
[![License](https://img.shields.io/badge/license-MPL--2.0-orange)](LICENSE)

## Table of Contents

- [Overview](#overview)
  * [Implementation Details](#implementation-details)
- [Documentation](#documentation)
- [Requirements](#requirements)
- [Installation](#installation)
  * [Getting the SDK](#getting-the-sdk)
  * [Running and Deployment](#running-and-deployment)
- [Usage](#usage)
  * [How to connect to QD endpoint](#how-to-connect-to-qd-endpoint)
  * [How to connect to dxLink](#how-to-connect-to-dxlink)
  * [Message Connectors](#message-connectors)
  * [System Properties](#system-properties)
  * [Threads and Listeners](#threads-and-listeners)
  * [Closing and Exit](#closing-and-exit)
- [Tools](#tools)
- [Samples](#samples)
- [Current State](#current-state)
- [How To Develop](HOW_TO_DEVELOP.md)
- [How To Release](HOW_TO_RELEASE.md), [Release Notes](ReleaseNotes.md)
- [License](LICENSE)

## Overview

dxFeed Graal Rust API lets Rust applications receive and publish dxFeed market data with a safe, idiomatic API:
owned handles released on drop, `Result` errors that carry Java exceptions, and closures as event listeners.
The names follow the [dxFeed Java API](https://docs.dxfeed.com/dxfeed/api/overview-summary.html): `DXEndpoint`,
`DXFeed`, `DXFeedSubscription` and `DXPublisher`, with methods in snake case (`add_event_listener`,
`close_and_await_termination`) including the getters (`get_feed`, `get_publisher`). Events print as Java prints them.

| Crate | Purpose |
|---|---|
| `dxfeed-graal-sys` | Raw FFI bindings to the SDK; the build script gets and links it |
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

As a result, we get a full-featured, similar performance as with Java API.
Regardless of the language, writing the final application logic using API calls will be very similar (only the syntax
will be amended, *"best practices"*, specific language restrictions).

## Documentation

Find useful information in our self-service dxFeed Knowledge Base or Rust API documentation:

- dxFeed Graal Rust API documentation: `cargo doc --open -p dxfeed-graal` (`--all-features` adds the
  [experimental tier](#getting-the-sdk))
- [dxFeed Knowledge Base](https://kb.dxfeed.com/index.html?lang=en)
  * [Getting Started](https://kb.dxfeed.com/en/getting-started.html)
  * [Troubleshooting](https://kb.dxfeed.com/en/troubleshooting-guidelines.html)
  * [Market Events](https://kb.dxfeed.com/en/data-model/dxfeed-api-market-events.html)
  * [Event Delivery contracts](https://kb.dxfeed.com/en/data-model/model-of-event-publishing.html#event-delivery-contracts)
  * [dxFeed API Event classes](https://kb.dxfeed.com/en/data-model/model-of-event-publishing.html#dxfeed-api-event-classes)
  * [Exchange Codes](https://kb.dxfeed.com/en/data-model/exchange-codes.html)
  * [Order Sources](https://kb.dxfeed.com/en/data-model/qd-model-of-market-events.html#order-x)
  * [Order Book reconstruction](https://kb.dxfeed.com/en/data-model/dxfeed-order-book/order-book-reconstruction.html)
  * [Symbology Guide](https://kb.dxfeed.com/en/data-model/symbology-guide.html)

## Requirements

### Rust

Rust 1.88 or newer, with one of the 64-bit targets `x86_64-pc-windows-msvc`, `x86_64-unknown-linux-gnu`,
`aarch64-unknown-linux-gnu`, `x86_64-apple-darwin` and `aarch64-apple-darwin`.

### Windows

Only x64 versions are supported.

| OS                                    | Version        | Architectures |
|---------------------------------------|----------------|---------------|
| [Windows][Windows-client]             | 8, 8.1         | x64           |
| [Windows 10][Windows-client]          | Version 1607+  | x64           |
| [Windows 11][Windows-client]          | Version 22000+ | x64           |
| [Windows Server][Windows-Server]      | 2012+          | x64           |
| [Windows Server Core][Windows-Server] | 2012+          | x64           |
| [Nano Server][Nano-Server]            | Version 1809+  | x64           |

#### Requirements

* [Visual C++ Redistributable for Visual Studio 2015][vc_redist] or newer (`VCRUNTIME140.dll`, `VCRUNTIME140_1.dll`)

[Windows-client]: https://www.microsoft.com/windows/

[Windows-Server]: https://learn.microsoft.com/windows-server/

[Nano-Server]: https://learn.microsoft.com/windows-server/get-started/getting-started-with-nano-server

[vc_redist]: https://aka.ms/vs/17/release/vc_redist.x64.exe

### Linux

x64 and aarch64 (arm64) versions are supported.

#### Libc compatibility

- [glibc][glibc]: 2.17+ (from Oracle Linux 7)
- [musl][musl]: not supported

#### Libpthread and libz compatibility

The SDK library needs `libpthread.so.0` and `libz.so.1`.

[glibc]: https://www.gnu.org/software/libc/

[musl]: https://musl.libc.org/

### macOS

| OS             | Version | Architectures |
|----------------|---------|---------------|
| [macOS][macOS] | 10.15+  | x64           |
| [macOS][macOS] | 11+     | Arm64         |

[macOS]: https://support.apple.com/macos

## Installation

Add the crate from [crates.io](https://crates.io/crates/dxfeed-graal):

```toml
[dependencies]
dxfeed-graal = "0.2"
```

The API is documented on [docs.rs](https://docs.rs/dxfeed-graal). `dxfeed-graal-build` deploys the SDK library next to
the executables of an application (see [Running and Deployment](#running-and-deployment)).

### Getting the SDK

The build script of `dxfeed-graal-sys` gets SDK 3.15.0 in one of these ways:

- `DXFEED_GRAAL_SDK_DIR` names an unpacked SDK archive (`graal-native-sdk-<version>-<platform>.zip` from the
  [release](https://github.com/dxFeed/dxfeed-graal-native-sdk/releases/tag/v3.15.0)). Use it for offline builds.
- Otherwise the default feature `download` fetches that archive from GitHub and checks its pinned SHA-256. The download
  honours the `ALL_PROXY`, `HTTPS_PROXY`, `HTTP_PROXY` and `NO_PROXY` environment variables and the operating system's
  certificate store. A timeout, a failed connection or an HTTP 429 or 5xx response is retried, three attempts in all.
- `DXFEED_GRAAL_SDK_CACHE_DIR`, an absolute path, keeps the downloaded archive in that directory, so a clean build or
  another target directory reuses it instead of downloading 21 MB again. The archive's SHA-256 is checked on every
  reuse; one that does not match is downloaded again with a warning. Without the variable nothing is written outside
  the build directory.
- With `--no-default-features` and no `DXFEED_GRAAL_SDK_DIR`, the build fails.

The build fails if the SDK headers differ from those the committed bindings were generated from.
`system::get_version()` returns the version of the loaded SDK library, `system::is_feature_supported` tells whether it
has an optional feature (the `system::feature` names), and `system::get_advanced_revision()` the revision of its
experimental functions, which matches `system::ADVANCED_REVISION` for the library this crate was built for.
The cargo feature `experimental`, off by default, adds the wrappers of the experimental tier of the SDK: the feed
filters (`feed_filter`), the ready signal of the event queues (`set_ready_signal`) and the event peek of the time
series transaction models (`with_event_peek`), the connector listener and the connection errors (see
[Message Connectors](#message-connectors)). They may change in a minor release of the SDK.
On docs.rs (`DOCS_RS` set) nothing is downloaded or linked.

### Running and Deployment

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
executable together with the library. Cargo examples and tests are not covered; they run through cargo.

For another layout, or when `build.build-dir` moves build scripts out of the target directory, call
`dxfeed_graal_build::library_path()` and copy the library yourself.
The [samples](samples) deploy the library this way (see [`samples/build.rs`](samples/build.rs)), and
`cargo run -p xtask -- check-deploy` checks that one of them runs from a directory holding only the executable and the
library.

## Usage

### How to connect to QD endpoint

```rust
use dxfeed_graal::DXEndpoint;
use dxfeed_graal::event::Quote;

fn main() -> dxfeed_graal::Result<()> {
    // For token-based authorization, use the following address format:
    // "demo.dxfeed.com:7300[login=entitle:token]"
    let endpoint = DXEndpoint::new_builder()
        .with_property("dxfeed.address", "demo.dxfeed.com:7300")
        .build()?;

    // Another way to connect the endpoint.
    // let endpoint = DXEndpoint::create()?.connect("demo.dxfeed.com:7300")?;

    let subscription = endpoint.get_feed().create_subscription::<Quote>()?;

    subscription.add_event_listener(|quotes| {
        for quote in quotes {
            println!("{quote}");
        }
    })?;

    subscription.add_symbols(["AAPL"])?;

    std::io::stdin().read_line(&mut String::new()).ok();
    endpoint.close_and_await_termination()
}
```

<details>
<summary>Output</summary>
<br>

```
I 261106 172639.536 [main] QDFactory - Using QDS-3.357, (C) Devexperts
I 261106 172639.563 [main] QDFactory - Using scheme com.dxfeed.api.impl.DXFeedScheme JFhzUYbcPr_gDabkoYd4_S0tWsvvKpzBmiN2aRCDnkY
I 261106 172639.606 [main] MARS - Started JVM self-monitoring
I 261106 172639.636 [main] MonitoredQDEndpoint - qdrst with collectors [Ticker, Stream, History]
...
Quote{AAPL, eventTime=0, time=20261106-172638.000, timeNanoPart=0, sequence=0, bidTime=20261106-172638.000, bidExchange=P, bidPrice=340.43, bidSize=120.0, askTime=20261106-172638.000, askExchange=Q, askPrice=340.95, askSize=160.0}
Quote{AAPL, eventTime=0, time=20261106-172640.000, timeNanoPart=0, sequence=0, bidTime=20261106-172638.000, bidExchange=P, bidPrice=340.43, bidSize=120.0, askTime=20261106-172640.000, askExchange=P, askPrice=340.95, askSize=80.0}
Quote{AAPL, eventTime=0, time=20261106-172640.000, timeNanoPart=0, sequence=0, bidTime=20261106-172640.000, bidExchange=P, bidPrice=340.52, bidSize=40.0, askTime=20261106-172640.000, askExchange=P, askPrice=340.95, askSize=80.0}
```

</details>

- A subscription to one event type, `create_subscription::<Quote>()`, gives its listener `&QuoteRef` values; a
  subscription to several, `create_subscription_with_types(&[EventType::Quote, EventType::Trade])`, gives `EventRef`
  values to match.
- Calls chain as in Java: a feed or a subscription taken from a chain keeps its endpoint open, e.g.
  `let feed = DXEndpoint::create()?.connect("demo.dxfeed.com:7300")?.get_feed();`.
- The last event of a lasting type (Quote, Profile, Summary, Greeks, Candle, Trade and others) comes from the local
  cache of the feed with `get_last_event_if_subscribed::<Quote>("AAPL")` (`None` without a subscription) and
  `get_last_event(&quote)`, or from the uplink with `get_last_event_promise::<Quote>("AAPL")?.wait(timeout)`. A wait
  that times out returns `Error::Timeout` and cancels the request; dropping a promise does not cancel it, as in Java.
- `DXFeed::get_instance()` and `DXEndpoint::get_instance()` return the default feed and endpoint of the process,
  configured on first use by the `dxfeed.properties` file and the system properties.

### How to connect to dxLink

```rust
use dxfeed_graal::event::Quote;
use dxfeed_graal::{DXEndpoint, system};

fn main() -> dxfeed_graal::Result<()> {
    // Enable the experimental feature.
    system::set_property("dxfeed.experimental.dxlink.enable", "true")?;
    // Set the scheme for dxLink.
    system::set_property("scheme", "ext:opt:sysprops,resource:dxlink.xml")?;

    // For token-based authorization, use the following address format:
    // "dxlink:wss://demo.dxfeed.com/market-data/dxlink-ws[login=dxlink:token]"
    let endpoint = DXEndpoint::new_builder()
        .with_property("dxfeed.address", "dxlink:wss://demo.dxfeed.com/market-data/dxlink-ws")
        .build()?;

    let subscription = endpoint.get_feed().create_subscription::<Quote>()?;

    subscription.add_event_listener(|quotes| {
        for quote in quotes {
            println!("{quote}");
        }
    })?;

    subscription.add_symbols(["AAPL"])?;

    std::io::stdin().read_line(&mut String::new()).ok();
    endpoint.close_and_await_termination()
}
```

To familiarize with the dxLink protocol, please click [here](https://demo.dxfeed.com/market-data/dxlink-ws/debug/#/protocol).

### Message Connectors

An endpoint has one message connector per address it is connected to, such as two for `(host1:7300)(host2:7300)`.
`DXEndpoint::get_connectors` lists them; each `MessageConnector` gives its name, kind, address with the credentials
masked, the addresses connected now, its state, the open and closed connections, and the bytes and data records read
and written:

```rust
use dxfeed_graal::DXEndpoint;

fn main() -> dxfeed_graal::Result<()> {
    let endpoint = DXEndpoint::create()?;
    endpoint.connect("demo.dxfeed.com:7300")?;
    std::thread::sleep(std::time::Duration::from_secs(5));
    for connector in endpoint.get_connectors()? {
        let statistics = connector.get_statistics()?;
        println!(
            "{} {} {} {:?}: {} bytes, {} data records read",
            connector.get_name()?,
            connector.get_address()?,
            connector.get_state()?,
            connector.get_current_addresses()?,
            statistics.read_bytes,
            statistics.read_data_records
        );
    }
    endpoint.close_and_await_termination()
}
```

A connector is a live view: each getter reads it anew, and it answers after a connect to another address replaced it
or its endpoint was closed, as `DISCONNECTED`. It does not keep its endpoint open.

With the cargo feature `experimental`, `DXEndpoint::add_message_connector_listener` calls a closure with a connector
after its state changes, and `MessageConnector::get_last_error` and `DXEndpoint::get_last_connection_error` return the
last failed connect or dropped connection: its time, category (`CONNECT_FAILED`, `HEARTBEAT_TIMEOUT` and others),
exception and message. The SDK calls all connector listeners on one thread of its own, so return quickly:

```rust
use dxfeed_graal::DXEndpoint;

fn main() -> dxfeed_graal::Result<()> {
    let endpoint = DXEndpoint::create()?;
    endpoint.add_message_connector_listener(|connector| {
        if let (Ok(address), Ok(state)) = (connector.get_address(), connector.get_state()) {
            println!("{address}: {state}");
        }
        if let Ok(Some(error)) = connector.get_last_error() {
            println!("last error: {} {:?}", error.category, error.message);
        }
    })?;
    endpoint.connect("demo.dxfeed.com:7300")?;
    std::io::stdin().read_line(&mut String::new()).ok();
    endpoint.close_and_await_termination()
}
```

### System Properties

As in the C++ and .NET APIs, the system properties are set when the SDK starts: first from the
`dxfeed.system.properties` file in the current directory, or the file named by the `DXFEED_dxfeed.system.properties`
environment variable, and then from the `DXFEED_<property>` environment variables, which override the file. The file
holds `key=value` lines; `#` and `;` start comments, also after a value:

```properties
dxfeed.address=demo.dxfeed.com:7300
```

The same property as an environment variable, in a POSIX shell and in PowerShell:

```bash
env DXFEED_dxfeed.address=demo.dxfeed.com:7300 ./my-app
```

```powershell
${env:DXFEED_dxfeed.address} = "demo.dxfeed.com:7300"
```

`system::set_property("dxfeed.address", ...)` sets a property in code. Most properties take effect only when they are
set before the first endpoint is created.

A `tls+` address checks the certificate of the server against the root CAs of the JDK that builds the SDK. To trust
other roots, set `javax.net.ssl.trustStore` to a trust store file (and `javax.net.ssl.trustStorePassword`, if it has
one) before the first TLS connection.

### Threads and Listeners

- All types are `Send` and `Sync`. Threads are attached to the SDK on their first call and detached when they exit.
- A listener's closure runs on a thread of the endpoint's executor, its own threads by default or an `Executor`
  thread pool (`Executor::new_fixed_thread_pool`), or inside `ManualExecutor::process_all_pending`.
  The calls of one listener do not overlap. The events are valid during the call only: copy what you keep with
  `to_owned()`. A panic in a closure is caught and goes to `ErrorManager`, as the C++ API does; the listener stays
  active.
- Errors without a caller (panics of listeners, data the API cannot read, failed releases of native data on drop) go
  to `ErrorManager`: its default handler writes one line per error to stderr, `ErrorManager::set_handler` replaces it,
  and a journal keeps the last errors (`get_errors`, `get_last_error`). The handler runs on the thread of the error;
  for a listener, inside its call, so like the listener it must not wait for a thread that removes that listener.
- Add listeners before symbols.
- Removing a listener or dropping its subscription waits for its running calls (inside a callback it does not), and so
  does dropping the last owner of an endpoint with state listeners. So do not hold a lock that a closure takes while
  doing that from another thread.

### Closing and Exit

- `DXEndpoint::close_and_await_termination()` closes an endpoint and waits for the notifications already scheduled;
  `close()` does not wait. Dropping the last handle of an endpoint (the endpoint, its feed, its publisher or a
  subscription) closes it as well. Close the event queues of its subscriptions first: it waits while a queue holds
  events that no poll returned.
- The process may exit with open endpoints and running listeners: from the moment it starts exiting, listeners are no
  longer called and destructors no longer call the SDK.
- `system::shutdown()` is an advanced function with no counterpart in the other dxFeed APIs. It stops all listeners,
  waiting for the running calls, and closes all endpoints of the process. Afterwards
  nothing new can be created (`Error::ShutDown`), while existing values can still be closed and dropped. It cannot be
  called from a listener (`Error::InCallback`). While it runs, an endpoint on a `ManualExecutor` needs another thread
  calling `process_all_pending`.
- Only executables are supported: a Rust `cdylib` loaded as a DLL is not.

## Tools

[Tools](tools/src/main.rs) is a collection of utilities that allow you to subscribe to various market events for the
specified symbols. It is one binary, `Tools`, whose first argument names the tool; `Tools Help <tool>` describes the
arguments of a tool, and `Tools Help contents` lists the help articles, such as the address format and the tape
options:

- [x] [Connect](tools/src/connect.rs) connects to the specified address(es) and subscribes to the specified events
  with the specified symbol
- [x] [Dump](tools/src/dump.rs) dumps all events received from address. This was designed to retrieve data from a
  file
- [x] [PerfTest](tools/src/perf_test.rs) connects to the specified address(es) and calculates performance counters
  (events per second, memory usage, CPU usage, etc.)
- [x] [LatencyTest](tools/src/latency_test.rs) connects to the specified address(es) and calculates latency
- [x] [Qds](tools/src/qds.rs) collection of tools ported from the Java qds-tools
- [x] [Help](tools/src/help.rs) shows the help screens of the tools and the help articles

Each [release](https://github.com/dxFeed/dxfeed-graal-rust-api/releases) has the Tools and Samples archives of every
platform (`dxFeedGraalRustApi-<version>-<arch>-<os>-Tools.zip|.tar.gz`), with the SDK library next to the executables
in `bin/`; the Linux ones need glibc 2.28 or newer. On macOS, remove the quarantine attribute of the downloaded files
first: `xattr -d -r com.apple.quarantine <unpacked folder>`.

Build and run it with cargo, or build it once and run `target/release/Tools`, which has the SDK library next to it
(see [Running and Deployment](#running-and-deployment)):

```shell
cargo run --release -p dxfeed-graal-tools -- Help contents
target/release/Tools Connect demo.dxfeed.com:7300 Quote AAPL,IBM
target/release/Tools Dump "file:quotes.txt[speed=max]" Quote -t quotes-copy.txt
target/release/Tools PerfTest demo.dxfeed.com:7300 Quote AAPL,IBM -p PerfTest.format=csv
target/release/Tools LatencyTest demo.dxfeed.com:7300 Quote,Trade AAPL,IBM --interval 5
target/release/Tools Qds "help connect"
```

## Samples

The samples follow the samples of the dxFeed Java API and have the names and groups of the samples of the C++ API.
Each one is a binary of the `dxfeed-graal-samples` package in `samples/<Group>/<Name>Sample`; started without its
required arguments, it prints its usage.

The Samples archive of a [release](https://github.com/dxFeed/dxfeed-graal-rust-api/releases) holds all of them built,
with the SDK library and `dxfeed.system.properties` in its `bin/` (see [Tools](#tools)).

A sample that uses the default feed reads its address from the `dxfeed.system.properties` file in the current
directory, so run it from its directory, which holds that file:

```bash
cd samples/API/PrintQuoteEventsSample
cargo run --bin PrintQuoteEventsSample -- AAPL
```

The `DXFEED_dxfeed.address` environment variable overrides the file (see [System Properties](#system-properties)):

```bash
env DXFEED_dxfeed.address=demo.dxfeed.com:7300 cargo run --bin PrintQuoteEventsSample -- AAPL
```

The build script of the samples puts the SDK library next to them, so a built sample also runs outside cargo
(see [Running and Deployment](#running-and-deployment)):

```bash
cargo build --release -p dxfeed-graal-samples
cd samples/API/PrintQuoteEventsSample
../../../target/release/PrintQuoteEventsSample AAPL
```

### API

- [x] [AuthSample](samples/API/AuthSample/main.rs)
  demonstrates how to connect to an endpoint that requires an authentication token, subscribe to market data events,
  and handle periodic token updates
- [x] [ConnectSample](samples/API/ConnectSample/main.rs)
  demonstrates how to connect to the endpoint and subscribe to various market events
- [x] [PrintQuoteEventsSample](samples/API/PrintQuoteEventsSample/main.rs)
  is a simple demonstration of how to subscribe to the `Quote` event, using the `DXFeed` instance singleton and the
  `dxfeed.system.properties` file
- [x] [PublishProfilesSample](samples/API/PublishProfilesSample/main.rs)
  is a simple demonstration of how to publish market events
- [x] [RequestProfileSample](samples/API/RequestProfileSample/main.rs)
  using an address like `localhost:7700` and a symbol like `A:TEST`, connects to the running `PublishProfilesSample`
  sample, prints the profile of the symbol and exits
- [x] [QuoteAndTradeSample](samples/API/QuoteAndTradeSample/main.rs)
  demonstrates how to create multiple event listeners and subscribe to `Quote` and `Trade` events
- [x] [ReconnectSample](samples/API/ReconnectSample/main.rs)
  demonstrates how to connect to an endpoint, subscribe to market data events, and handle reconnections and
  re-subscribing

### Candle

- [x] [CandleSample](samples/Candle/CandleSample/main.rs)
  demonstrates how to subscribe to `Candle` events from a time with the candle symbol attributes
- [x] [CandleDataSample](samples/Candle/CandleDataSample/main.rs)
  demonstrates how to query the CandleData service using the `HistoryEndpoint`
- [x] [FetchDailyCandlesSample](samples/Candle/FetchDailyCandlesSample/main.rs)
  demonstrates how to fetch the candles of the last N days for a symbol

### Console

- [x] [LastEventConsoleSample](samples/Console/LastEventConsoleSample/main.rs)
  demonstrates how to subscribe to various market events with the dxFeed API, cache them in memory, and take
  snapshots of these events based on user input

### File

- [x] [ConvertTapeFileSample](samples/File/ConvertTapeFileSample/main.rs)
  demonstrates how to convert one tape file to another tape file with optional intermediate processing or filtering
- [x] [FileParserSample](samples/File/FileParserSample/main.rs)
  demonstrates how to read events from a tape file
- [x] [WriteTapeFileSample](samples/File/WriteTapeFileSample/main.rs)
  demonstrates how to write events to a tape file

### IPF

- [x] [IpfSample](samples/IPF/IpfSample/main.rs)
  is a simple demonstration of how to get instrument profiles
- [x] [IpfLiveSample](samples/IPF/IpfLiveSample/main.rs)
  is a simple demonstration of how to get live updates of instrument profiles
- [x] [OptionChainSample](samples/IPF/OptionChainSample/main.rs)
  demonstrates how to build option chains, and prints quotes for nearby option strikes

### Model

- [x] [SimpleOrderBookSample](samples/Model/SimpleOrderBookSample/main.rs)
  demonstrates how to build an order book of a symbol and an order source with `IndexedEventTxModel`
- [x] [IncOrderSnapshotSample](samples/Model/IncOrderSnapshotSample/main.rs)
  demonstrates how to subscribe to the `Order` event and handle snapshots and updates
- [x] [MultipleMarketDepthSample](samples/Model/MultipleMarketDepthSample/main.rs)
  demonstrates how to use the `MarketDepthModel` to manage and display order books for multiple symbols
- [x] [PriceLevelBookSample](samples/Model/PriceLevelBookSample/main.rs)
  demonstrates how to compile a price level book (market by price) that aggregates individual orders (market by
  order)
- [x] [MarketDepthModelSample](samples/Model/MarketDepthModelSample/main.rs)
  demonstrates how to use the `MarketDepthModel`

### Schedule

- [x] [ScheduleSample](samples/Schedule/ScheduleSample/main.rs)
  demonstrates how to get various scheduling information for instruments

### OnDemand

- [x] [OnDemandSample](samples/OnDemand/OnDemandSample/main.rs)
  demonstrates how to use the dxFeed on-demand history data replay service API

### ORCS

- [x] [PriceLevelServiceClientSample](samples/Orcs/PriceLevelServiceClientSample/main.rs)
  demonstrates how to get the price levels and the quotes of a symbol for a period from the Order Reconstruction
  Service (ORCS) with `PriceLevelService`

## Current State

The API covers the dxFeed Java API 3.357 wherever the SDK exports it, its experimental tier with the feature
`experimental`. `PriceLevelService.getCandles` and `PriceLevelCandle` wait for SDK exports. Deprecated Java members are
left out.

### Endpoint Roles

- [x] [FEED](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/api/DXEndpoint.Role.html#FEED) (`Role::Feed`) connects to
  the remote data feed provider and is optimized for real-time or delayed data processing, this is a default role
- [x] [STREAM_FEED](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/api/DXEndpoint.Role.html#STREAM_FEED)
  (`Role::StreamFeed`) is similar to FEED and also connects to the remote data feed provider but is designed for bulk
  data parsing from files
- [x] [PUBLISHER](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/api/DXEndpoint.Role.html#PUBLISHER)
  (`Role::Publisher`) connects to the remote publisher hub (also known as multiplexor) or creates a publisher on the
  local host ([WriteTapeFileSample](samples/File/WriteTapeFileSample/main.rs))
- [x] [STREAM_PUBLISHER](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/api/DXEndpoint.Role.html#STREAM_PUBLISHER)
  (`Role::StreamPublisher`) is similar to PUBLISHER and also connects to the remote publisher hub, but is designed for
  bulk data publishing
- [x] [LOCAL_HUB](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/api/DXEndpoint.Role.html#LOCAL_HUB)
  (`Role::LocalHub`) is a local hub without the ability to establish network connections. Events published via
  publisher are delivered to local feed only
- [x] [ON_DEMAND_FEED](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/api/DXEndpoint.Role.html#ON_DEMAND_FEED)
  (`Role::OnDemandFeed`) is similar to FEED, but it is designed to be used with `OnDemandService` for historical data
  replay only

### Event Types

- [x] [Order](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/market/Order.html) is a snapshot of the full
  available market depth for a symbol
- [x] [SpreadOrder](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/market/SpreadOrder.html) is a snapshot of the
  full available market depth for all spreads
- [x] [AnalyticOrder](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/market/AnalyticOrder.html) represents an
  extension of Order introducing analytic information, e.g., adding iceberg-related information to this order
- [x] [OrderBase](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/market/OrderBase.html) holds the common fields
  of the order events; it is received only, as Java refuses to publish it
- [x] [Trade](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/market/Trade.html) is a snapshot of the price and
  size of the last trade during regular trading hours and an overall day volume and day turnover
- [x] [TradeETH](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/market/TradeETH.html) is a snapshot of the price
  and size of the last trade during extended trading hours and the extended trading hours day volume and day turnover
- [x] [Candle](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/candle/Candle.html) - event with open, high, low,
  and close prices and other information for a specific period
- [x] [DailyCandle](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/candle/DailyCandle.html) (deprecated in Java)
  is the former daily candle with additional properties, which have moved to `Candle`
- [x] [Quote](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/market/Quote.html) is a snapshot of the best bid and
  ask prices and other fields that change with each quote
- [x] [Profile](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/market/Profile.html) is a snapshot that contains
  the security instrument description
- [x] [Summary](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/market/Summary.html) is a snapshot of the trading
  session, including session highs, lows, etc.
- [x] [TimeAndSale](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/market/TimeAndSale.html) - represents a trade
  or other market event with price, like market open/close price, etc.
- [x] [Greeks](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/option/Greeks.html) is a snapshot of the option
  price, Black-Scholes volatility, and Greeks
- [x] [Series](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/option/Series.html) is a snapshot of computed
  values available for all options series for a given underlying symbol based on options market prices
- [x] [TheoPrice](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/option/TheoPrice.html) is a snapshot of the
  theoretical option price computation that is periodically performed
  by [dxPrice](http://www.devexperts.com/en/products/price.html) model-free computation
- [x] [Underlying](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/option/Underlying.html) is a snapshot of
  computed values available for an option underlying symbol based on the market’s option prices
- [x] [Message](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/misc/Message.html) is an event with an
  application-specific attachment
- [x] [TextMessage](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/misc/TextMessage.html) is a message event
  with a text payload
- [x] [Configuration](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/misc/Configuration.html) is a configuration
  event with an application-specific attachment and a version
- [x] [TextConfiguration](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/misc/TextConfiguration.html) is a
  configuration event with a text payload and a version
- [x] [OptionSale](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/market/OptionSale.html) is an event that
  represents a trade or another market event with the price (for example, market open/close price, etc.) for each
  option symbol listed under the specified Underlying
- [x] [OtcMarketsOrder](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/market/OtcMarketsOrder.html) represents
  an extension of `Order` for the symbols traded on the OTC Markets
- [x] [OrderImbalance](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/market/OrderImbalance.html) (experimental)
  shows the imbalance between buy and sell orders during auctions
- [x] [NuamOrder](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/custom/NuamOrder.html) (experimental) is an
  extension of `Order` for the NUAM exchange
- [x] [NuamTrade](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/custom/NuamTrade.html) (experimental) is an
  extension of `Trade` for the NUAM exchange
- [x] [NuamTimeAndSale](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/custom/NuamTimeAndSale.html)
  (experimental) is an extension of `TimeAndSale` for the NUAM exchange
- [x] [MarketMaker](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/market/MarketMaker.html) is the bid and ask of
  a market maker on an exchange

All of them print in the format of Java's `toString`, and each has the event symbol and the event time, owned
(`Event`) or borrowed (`EventRef` and the views). `EventType::get_name`, `FromStr` and `EventType::ALL` name and list
the types.

### Subscription Symbols

- [x] String is a string representation of the symbol (`Symbol::String`)
- [x] [TimeSeriesSubscriptionSymbol](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/api/osub/TimeSeriesSubscriptionSymbol.html)
  represents subscription to time-series events (`Symbol::time_series`)
- [x] [IndexedEventSubscriptionSymbol](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/api/osub/IndexedEventSubscriptionSymbol.html)
  represents subscription to a specific source of indexed events (`Symbol::indexed`), with the order sources
- [x] [WildcardSymbol.ALL](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/api/osub/WildcardSymbol.html#ALL) represents
  a *wildcard* subscription to all events of the specific event type (`Symbol::Wildcard`)
- [x] [CandleSymbol](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/candle/CandleSymbol.html) symbol used with
  `DXFeedSubscription` to subscribe for `Candle` events, with all its attributes, `CandlePeriod` parsing and
  `MarketEventSymbols`

### Subscriptions & Models

- [x] [DXFeedSubscription](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/api/DXFeedSubscription.html) is a
  subscription for a set of symbols and event types: typed and untyped, with listeners, change listeners (with
  `configuration_changed`), the symbols
  (`set_symbols`, `get_symbols`, `get_decorated_symbols`, `clear`), the aggregation period, the events batch limit,
  dropped events, executors, and standalone subscriptions attached to feeds; `DXFeedTimeSeriesSubscription` for time
  series
- [x] [DXPublisher](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/api/DXPublisher.html) publishes events, and its
  observable subscription tells which symbols are subscribed
- [x] [GetLastEvent](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/api/DXFeed.html#getLastEvent-E-)
  returns the last event for the specified event instance; also `get_last_event_if_subscribed`, `get_last_events`,
  `get_last_event_promise` and `get_last_events_promises`
- [x] [GetTimeSeriesPromise](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/api/DXFeed.html#getTimeSeriesPromise-java.lang.Class-java.lang.Object-long-long-)
  requests time series events for the specified event type, symbol, and time range; also
  `get_indexed_events_promise`
- [x] [Promise](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/promise/Promise.html) with `when_done`,
  `when_done_async` and `complete`, `Promises::all_of` and `Promises::any_of`; promises created in Rust with
  `Promise::new` and `Promise::failed`, failed with `complete_exceptionally` and a `JavaExceptionClass`
- [x] Event queues: the events of subscriptions polled on a thread of the application (`EventQueue`), and a
  `SharedEventQueue` with tags whose executor runs the models, the collector and the promise handlers in the polls
- [x] Transactional models of indexed and time-series events: `IndexedTxModel`, `TimeSeriesTxModel`,
  `IndexedEventTxModel`
- [x] Market depth model of orders: `MarketDepthModel`
- [x] List models: `OrderBookModel`, `IndexedEventModel`, `TimeSeriesEventModel` and their `ObservableListModel`

### IPF & Schedule

- [x] [InstrumentProfile](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/ipf/InstrumentProfile.html) represents basic
  profile information about a market instrument, with typed fields
  ([IpfSample](samples/IPF/IpfSample/main.rs))
- [x] [InstrumentProfileReader](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/ipf/InstrumentProfileReader.html) reads
  instrument profiles from files, URLs and bytes using Instrument Profile Format (IPF)
- [x] [InstrumentProfileWriter](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/ipf/InstrumentProfileWriter.html)
  writes instrument profiles to files and bytes
- [x] [InstrumentProfileConnection](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/ipf/live/InstrumentProfileConnection.html)
  connects to an instrument profile URL and reads instrument profiles with support of streaming live updates
- [x] [InstrumentProfileCollector](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/ipf/live/InstrumentProfileCollector.html)
  collects instrument profile updates and provides the live instrument profiles list, with generations
  ([IpfLiveSample](samples/IPF/IpfLiveSample/main.rs))
- [x] [OptionChainsBuilder](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/ipf/option/OptionChainsBuilder.html) builds
  option chains ([OptionChainSample](samples/IPF/OptionChainSample/main.rs))
- [x] The glossary: `PriceIncrements`, `Cfi` and additional underlyings
- [x] [Schedule](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/schedule/Schedule.html) provides API to retrieve and
  explore various exchanges’ trading schedules and different financial instrument classes: days, sessions, holidays,
  venues, the schedule defaults, UTC and NYSE, the `accept` of the day and session filters, and custom filters with a Rust predicate
  ([ScheduleSample](samples/Schedule/ScheduleSample/main.rs))

### Services

- [x] [OnDemandService](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/ondemand/OnDemandService.html) provides on-demand
  historical tick data replay controls, with property change listeners
  ([OnDemandSample](samples/OnDemand/OnDemandSample/main.rs))
- [x] `PriceLevelService` gets the price levels and the quotes of the Order Reconstruction Service (ORCS), with
  `PriceLevelChecker` and `AuthOrderSource`
  ([PriceLevelServiceClientSample](samples/Orcs/PriceLevelServiceClientSample/main.rs))
- [x] `HistoryEndpoint` gets candles and time and sales from the history service
  ([CandleDataSample](samples/Candle/CandleDataSample/main.rs))

### Other Features

- [x] System properties, the `dxfeed.system.properties` file and `DXFEED_*` environment variables, the default
  endpoint and feed (`get_instance`)
- [x] Endpoint properties, states and state listeners, `reconnect`, `disconnect_and_clear`, `await_processed`,
  `await_not_connected`, user and password, `AuthToken`, event types and aggregation period info
- [x] [Message connectors](#message-connectors) of an endpoint: their addresses, states, connections and statistics
- [x] `TimeFormat`, `TimePeriod`, `TimeZone`, the logging bridge (`logging`) and executors with thread pools
- [x] `close_and_await_termination`, explicit shutdown and clean process exit
- [x] Errors without a caller: `ErrorManager` with a handler and a journal
- [x] Deployment of the SDK library next to executables, a download cache for the SDK archive
- [x] The [Tools](#tools) binary and the QDS tools of the SDK (`qds::Tools`): the symbol list parser and the
  qds-tools runner
- [x] With the cargo feature `experimental`: feed filters (`DXFeedFilter`, its builder and its tracker with
  listeners), the ready signal of the event queues (an `eventfd` or a Windows event), the event peek of
  `TimeSeriesTxModel`, the message connector listener and the last connection errors
