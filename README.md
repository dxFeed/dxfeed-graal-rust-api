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
![Graal Native SDK](https://img.shields.io/badge/Graal%20Native%20SDK-v3.13.0-yellow)
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
  * [System Properties](#system-properties)
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
`close_and_await_termination`) including the getters (`get_feed`, `get_publisher`). Events print as Java prints them.

| Crate | Purpose |
|---|---|
| `dxfeed-graal-sys` | Raw FFI bindings to SDK 3.13.0; the build script gets and links the SDK |
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

The build script of `dxfeed-graal-sys` gets SDK 3.13.0 in one of these ways:

- `DXFEED_GRAAL_SDK_DIR` names an unpacked SDK archive (`graal-native-sdk-3.13.0-<platform>.zip` from the
  [release](https://github.com/dxFeed/dxfeed-graal-native-sdk/releases/tag/v3.13.0)). Use it for offline builds.
- Otherwise the default feature `download` fetches that archive from GitHub and checks its pinned SHA-256. The download
  honours the `ALL_PROXY`, `HTTPS_PROXY`, `HTTP_PROXY` and `NO_PROXY` environment variables and the operating system's
  certificate store.
- `DXFEED_GRAAL_SDK_CACHE_DIR`, an absolute path, keeps the downloaded archive in that directory, so a clean build or
  another target directory reuses it instead of downloading 21 MB again. The archive's SHA-256 is checked on every
  reuse; one that does not match is downloaded again with a warning. Without the variable nothing is written outside
  the build directory.
- With `--no-default-features` and no `DXFEED_GRAAL_SDK_DIR`, the build fails.

The build fails if the SDK headers differ from those of 3.13.0, because the committed bindings fit only that version.
`system::get_version()` returns the version of the loaded SDK library, `system::is_feature_supported` tells whether it
has an optional feature (the `system::feature` names), and `system::get_advanced_revision()` the revision of its
experimental functions, which matches `system::ADVANCED_REVISION` for the library this crate was built for. The
crate does not wrap the event queue, the feed filters and the event peek of these features yet.
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
    let subscription = endpoint.get_feed().create_subscription::<Quote>()?;
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
  `let feed = DXEndpoint::create()?.connect("demo.dxfeed.com:7300")?.get_feed();`.
- The last event of a lasting type (Quote, Profile, Summary, Greeks, Candle, Trade and others) comes from the local
  cache of the feed with `get_last_event_if_subscribed::<Quote>("AAPL")` (`None` without a subscription) and
  `get_last_event(&quote)`, or from the uplink with `get_last_event_promise::<Quote>("AAPL")?.wait(timeout)`. A wait
  that times out returns `Error::Timeout` and cancels the request; dropping a promise does not cancel it, as in Java.
- `DXFeed::get_instance()` and `DXEndpoint::get_instance()` return the default feed and endpoint of the process,
  configured on first use by the `dxfeed.properties` file and the system properties.

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

## Samples

The samples follow the samples of the dxFeed Java API and have the names and groups of the samples of the C++ API.
Each one is a Cargo example in `dxfeed-graal/examples/<Group>/<Name>Sample`; started without its required arguments,
it prints its usage.

A sample that uses the default feed reads its address from the `dxfeed.system.properties` file in the current
directory, so run it from its directory, which holds that file:

```bash
cd dxfeed-graal/examples/API/PrintQuoteEventsSample
cargo run --example PrintQuoteEventsSample -- AAPL
```

The `DXFEED_dxfeed.address` environment variable overrides the file (see [System Properties](#system-properties)):

```bash
env DXFEED_dxfeed.address=demo.dxfeed.com:7300 cargo run --example PrintQuoteEventsSample -- AAPL
```

### API

- [x] [AuthSample](dxfeed-graal/examples/API/AuthSample/main.rs)
  demonstrates how to connect to an endpoint that requires an authentication token, subscribe to market data events,
  and handle periodic token updates
- [x] [ConnectSample](dxfeed-graal/examples/API/ConnectSample/main.rs)
  demonstrates how to connect to the endpoint and subscribe to various market events
- [x] [PrintQuoteEventsSample](dxfeed-graal/examples/API/PrintQuoteEventsSample/main.rs)
  is a simple demonstration of how to subscribe to the `Quote` event, using the `DXFeed` instance singleton and the
  `dxfeed.system.properties` file
- [x] [PublishProfilesSample](dxfeed-graal/examples/API/PublishProfilesSample/main.rs)
  is a simple demonstration of how to publish market events
- [x] [RequestProfileSample](dxfeed-graal/examples/API/RequestProfileSample/main.rs)
  using an address like `localhost:7700` and a symbol like `A:TEST`, connects to the running `PublishProfilesSample`
  sample, prints the profile of the symbol and exits
- [x] [QuoteAndTradeSample](dxfeed-graal/examples/API/QuoteAndTradeSample/main.rs)
  demonstrates how to create multiple event listeners and subscribe to `Quote` and `Trade` events
- [x] [ReconnectSample](dxfeed-graal/examples/API/ReconnectSample/main.rs)
  demonstrates how to connect to an endpoint, subscribe to market data events, and handle reconnections and
  re-subscribing

### Candle

- [x] [CandleSample](dxfeed-graal/examples/Candle/CandleSample/main.rs)
  demonstrates how to subscribe to `Candle` events from a time with the candle symbol attributes
- [x] [CandleDataSample](dxfeed-graal/examples/Candle/CandleDataSample/main.rs)
  demonstrates how to query the CandleData service using the `HistoryEndpoint`
- [x] [FetchDailyCandlesSample](dxfeed-graal/examples/Candle/FetchDailyCandlesSample/main.rs)
  demonstrates how to fetch the candles of the last N days for a symbol

### Console

- [x] [LastEventConsoleSample](dxfeed-graal/examples/Console/LastEventConsoleSample/main.rs)
  demonstrates how to subscribe to various market events with the dxFeed API, cache them in memory, and take
  snapshots of these events based on user input

### File

- [x] [ConvertTapeFileSample](dxfeed-graal/examples/File/ConvertTapeFileSample/main.rs)
  demonstrates how to convert one tape file to another tape file with optional intermediate processing or filtering
- [x] [FileParserSample](dxfeed-graal/examples/File/FileParserSample/main.rs)
  demonstrates how to read events from a tape file
- [x] [WriteTapeFileSample](dxfeed-graal/examples/File/WriteTapeFileSample/main.rs)
  demonstrates how to write events to a tape file

### IPF

- [x] [IpfSample](dxfeed-graal/examples/IPF/IpfSample/main.rs)
  is a simple demonstration of how to get instrument profiles
- [x] [IpfLiveSample](dxfeed-graal/examples/IPF/IpfLiveSample/main.rs)
  is a simple demonstration of how to get live updates of instrument profiles
- [x] [OptionChainSample](dxfeed-graal/examples/IPF/OptionChainSample/main.rs)
  demonstrates how to build option chains, and prints quotes for nearby option strikes

### Model

- [x] [SimpleOrderBookSample](dxfeed-graal/examples/Model/SimpleOrderBookSample/main.rs)
  demonstrates how to build an order book of a symbol and an order source with `IndexedEventTxModel`
- [x] [IncOrderSnapshotSample](dxfeed-graal/examples/Model/IncOrderSnapshotSample/main.rs)
  demonstrates how to subscribe to the `Order` event and handle snapshots and updates
- [x] [MultipleMarketDepthSample](dxfeed-graal/examples/Model/MultipleMarketDepthSample/main.rs)
  demonstrates how to use the `MarketDepthModel` to manage and display order books for multiple symbols
- [x] [PriceLevelBookSample](dxfeed-graal/examples/Model/PriceLevelBookSample/main.rs)
  demonstrates how to compile a price level book (market by price) that aggregates individual orders (market by
  order)
- [x] [MarketDepthModelSample](dxfeed-graal/examples/Model/MarketDepthModelSample/main.rs)
  demonstrates how to use the `MarketDepthModel`

### Schedule

- [x] [ScheduleSample](dxfeed-graal/examples/Schedule/ScheduleSample/main.rs)
  demonstrates how to get various scheduling information for instruments

### OnDemand

- [x] [OnDemandSample](dxfeed-graal/examples/OnDemand/OnDemandSample/main.rs)
  demonstrates how to use the dxFeed on-demand history data replay service API

### ORCS

- [x] [PriceLevelServiceClientSample](dxfeed-graal/examples/Orcs/PriceLevelServiceClientSample/main.rs)
  demonstrates how to get the price levels and the quotes of a symbol for a period from the Order Reconstruction
  Service (ORCS) with `PriceLevelService`

[deploy](samples/deploy) is an application whose build script deploys the SDK library next to its executable (see
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
- [x] [TimeAndSale](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/market/TimeAndSale.html)
- [x] [NuamTimeAndSale](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/custom/NuamTimeAndSale.html)
- [x] [OptionSale](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/market/OptionSale.html)
- [x] [Candle](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/candle/Candle.html)
- [x] [DailyCandle](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/candle/DailyCandle.html)
- [x] [Order](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/market/Order.html)
- [x] [AnalyticOrder](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/market/AnalyticOrder.html)
- [x] [OtcMarketsOrder](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/market/OtcMarketsOrder.html)
- [x] [SpreadOrder](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/market/SpreadOrder.html)
- [x] [NuamOrder](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/custom/NuamOrder.html)
- [x] [MarketMaker](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/market/MarketMaker.html)
- [x] [OrderImbalance](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/market/OrderImbalance.html)
- [x] [OrderBase](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/market/OrderBase.html): received
  only; Java refuses to publish it
- [x] [Message](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/misc/Message.html)
- [x] [Configuration](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/misc/Configuration.html)
- [x] [TextMessage](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/misc/TextMessage.html)
- [x] [TextConfiguration](https://docs.dxfeed.com/dxfeed/api/com/dxfeed/event/misc/TextConfiguration.html)

### Features

- [x] System properties, the `dxfeed.system.properties` file and `DXFEED_*` environment variables, the default
  endpoint and feed (`get_instance`)
- [x] Typed and untyped subscriptions, listeners, publishing
- [x] Event queues: the events of subscriptions polled on a thread of the application (`EventQueue`), and a
  `SharedEventQueue` with tags whose executor runs the models, the collector and the promise handlers in the polls
- [x] Subscription symbols and state: `set_symbols`, `get_symbols`, `get_decorated_symbols`, `clear`, `is_closed`,
  event types, the aggregation period, the events batch limit, dropped events, executors and standalone
  subscriptions attached to feeds
- [x] The event symbol and the event time of any event, owned (`Event`) or borrowed (`EventRef` and the views)
- [x] Observable subscriptions of publishers and change listeners of subscriptions, with `configuration_changed`
- [x] Symbols: strings, candle symbols with their attributes, the wildcard, indexed event sources and time series
- [x] Time series subscriptions, `get_time_series_promise` and `get_indexed_events_promise`
- [x] `TimeFormat`: parsing and formatting of times like the other APIs
- [x] Last events: `get_last_event_if_subscribed`, `get_last_event`, `get_last_events`, `get_last_event_promise`
  and `get_last_events_promises`
- [x] Promises: `Promise` with `when_done`, `when_done_async` and `complete`, `Promises::all_of` and
  `Promises::any_of`; promises created in Rust with `Promise::new` and `Promise::failed`, failed with
  `complete_exceptionally` and a `JavaExceptionClass`
- [x] Endpoint properties, roles and the default instances, user and password, `AuthToken`, event types and
  aggregation period info
- [x] `TimePeriod`, `TimeZone`, the logging bridge (`logging`) and executors with thread pools
- [x] Events printed in the format of Java's `toString`
- [x] `close_and_await_termination`, explicit shutdown and clean process exit
- [x] Endpoint states and state listeners, `reconnect`, `disconnect_and_clear`, `await_processed`, `await_not_connected`
- [x] Event type names: `EventType::get_name`, parsing with `FromStr`, `EventType::ALL`
- [x] Deployment of the SDK library next to executables, a download cache for the SDK archive
- [x] Instrument profiles: reading files, URLs and bytes, writing files and bytes with `InstrumentProfileWriter`,
  typed fields, a collector with generations, a live connection and option chains
- [x] The glossary: `PriceIncrements`, `Cfi` and additional underlyings
- [x] Candle symbols with all their attributes, `CandlePeriod` parsing, `MarketEventSymbols`, order sources
- [x] Trading schedules: days, sessions, holidays, venues, the schedule defaults, UTC and NYSE, the `accept` of
  the day and session filters, and custom filters with a Rust predicate
- [x] Transactional models of indexed and time-series events: `IndexedTxModel`, `TimeSeriesTxModel`,
  `IndexedEventTxModel`
- [x] Market depth model of orders: `MarketDepthModel`
- [x] List models: `OrderBookModel`, `IndexedEventModel`, `TimeSeriesEventModel` and their `ObservableListModel`
- [x] On-demand historical data replay: `OnDemandService` and its property change listeners
- [x] The price levels and the quotes of the Order Reconstruction Service: `PriceLevelService`, `PriceLevelChecker`,
  `AuthOrderSource` (also created with `AuthOrderSource::create`)
- [x] Candles and time and sales from the history service: `HistoryEndpoint`
- [x] Errors without a caller: `ErrorManager` with a handler and a journal

The API covers the dxFeed Java API 3.356 wherever SDK 3.13.0 exports it, except these Java members, new in SDK
3.13.0, which wait for their Rust wrappers: the `DXFeedFilter` family.
`PriceLevelService.getCandles` and `PriceLevelCandle` wait for SDK exports. Deprecated Java members are left out.

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

The tests expect no `DXFEED_*` environment variables: the SDK reads them as system properties in every test process.

## License

MPL-2.0, see [LICENSE](LICENSE).
