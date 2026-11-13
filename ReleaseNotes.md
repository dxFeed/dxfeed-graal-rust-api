## v0.4.0

* **\[DXFR-81]\[Rust]** Promises can be awaited in async code.
    * `Promise` and `AnyOf` implement `IntoFuture` (`PromiseFuture`, `AnyOfFuture`), so async code awaits them instead
      of blocking a thread in `wait`. The futures work on any executor (tokio, async-std, smol or another); the crate
      depends on no runtime and needs no cargo feature for them.
    * `PromiseFuture::cancel` makes the future ready at once, so a timer of the runtime that cancels it is the async
      timeout. Dropping the future does not cancel the request.
    * `system::shutdown` makes the pending futures ready with `Error::ShutDown`.
* **\[DXFR-82]\[Rust]** Subscriptions can be consumed as async streams of owned events.
    * `DXFeedSubscription::into_stream` and `DXFeedTimeSeriesSubscription::into_stream` turn a subscription into an
      `EventStream`. Its buffer drops and counts the oldest events when it is full; `into_stream_with` takes
      `StreamOptions::bounded` with another capacity or `StreamOptions::unbounded`.
    * The stream ends when it or its subscription is closed, when all its endpoints are closed and on
      `system::shutdown`.
    * The new cargo feature `stream` implements `futures_core::Stream` for the streams; without it they have their own
      `next` and `poll_next`.
* **\[DXFR-83]\[Rust]** The crate docs index common tasks and give examples of subscriptions, symbols, promises, the
  transaction models and the main event types; the crate docs and the README have an Async section.
* **\[DXFR-84]\[Rust]** Event streams can take their events from an event queue.
    * With `StreamOptions::event_queue`, the events stay in the SDK until a task waits for them, so the SDK conflates
      the lasting events (the last quote of each symbol) while the consumer lags; `dropped()` counts what the SDK lost.
    * An endpoint that awaits its termination (`close_and_await_termination`) does not wait for the consumer: the
      stream takes what the queue holds.
* **\[DXFR-85]\[Rust]** The transactional models and the order books can be consumed as async streams.
    * `build_stream` of the builders of `IndexedTxModel` and `TimeSeriesTxModel` makes a `ModelStream` of
      `Transaction`s, one per transaction or snapshot.
    * `build_stream` of `MarketDepthModelBuilder` and `OrderBookModel::into_stream` make a `ModelStream` of
      `BookSides`, the whole book after a change.
    * A model stream never splits an item: when its consumer lags, a snapshot drops the buffered transactions of its
      source and a book replaces the buffered book; otherwise the buffer grows. It ends as an event stream does.
* **\[DXFR-86]\[Rust]** A shared event queue can be consumed as an async stream.
    * `SharedEventQueue::into_stream` makes a `TaggedEventStream` of `(tag, Event)`: the events of all the
      subscriptions attached to the queue, taken only while a task waits. The work of the executor of the queue
      (models, the collector, promise handlers) runs in those polls.
    * The stream does not own the subscriptions. Closing an endpoint does not end it, as its subscriptions may come
      from any endpoint: it ends when it or its queue is closed and on `system::shutdown`.

### For integrators

* Depend on `dxfeed-graal = "0.4"`. The crates still use dxFeed Graal Native SDK 3.16.0: `dxfeed-graal-sys` is
  `0.4.0+sdk.3.16.0` and its bindings do not change.

## v0.3.0

* **\[DXFR-80]\[Rust]** Migrated to dxFeed Graal Native SDK v3.16.0.
    * A handler of `logging::set_handler` works with `logging::set_log_level` at `Trace` and `All`: it gets the Java
      level `FINER` as `Trace` and `CONFIG` as `Debug`. Before, a call that logged at `FINER`, such as
      `DXEndpoint::connect`, failed with a `NullPointerException` while a handler was set.
    * The SDK deprecates `dxfg_v4_DXEndpoint_getInstanceWithRole`, which creates a new endpoint on every call, and
      drops it in SDK 4.0.0. `DXEndpoint::get_instance_with_role` does not call it: it returns the default endpoint of
      the SDK for `Role::Feed` and keeps one endpoint per other role, created with
      `dxfg_v4_DXEndpoint_createWithRole`.
    * `dxfg_v4_DXEndpoint_Builder_withProperties` of the SDK now closes the file it reads.
      `DXEndpointBuilder::with_properties` sets the properties one by one and does not use it.
    * The bindings change only in their doc comments.

### For integrators

* Depend on `dxfeed-graal = "0.3"`; `dxfeed-graal-sys` is `0.3.0+sdk.3.16.0`.

## v0.2.1

* **\[DXFR-79]\[Rust]** Migrated to dxFeed Graal Native SDK v3.15.0, built with GraalVM 25.5 (JDK 25.0.5). The API
  is the same as in 0.2.0.
    * The SDK trusts the root CAs of JDK 25.0.5 unless `javax.net.ssl.trustStore` is set: it no longer trusts the
      eMudhra and Comodo roots that this JDK removes and rejects the blocked eMudhra TLS CAs G3B. The macOS x86-64
      library is still built with JDK 25.0.1, with the trust store of GraalVM 25.5 but without the distrust rules that
      later JDKs implement in code (the Chunghwa ePKI Root CA).
    * The headers are those of SDK 3.14.0, so the bindings do not change.
    * A handler of `logging::set_handler` still cannot take the levels `Trace` and `All`.

### For integrators

* A connection to a server whose certificate chains up to one of the removed roots now fails the TLS handshake.
  `dxfeed-graal-sys` is `0.2.1+sdk.3.15.0`.

## v0.2.0

* **\[DXFR-74]\[Rust]** The published manifests drop `license-file` next to the SPDX license, the packages of
  `dxfeed-graal` and `dxfeed-graal-sys` include their tests, and docs.rs documents all five supported targets.
* **\[DXFR-75]\[Rust]** Migrated to dxFeed Graal Native SDK v3.14.0.
    * The SDK sets its root logger to the more verbose of the levels of `logging::set_log_level` (`INFO` by default)
      and `set_err_level`, so the messages below it are not created.
    * Closing a live connection of instrument profiles ends its thread with the next data or the read timeout;
      before, the end of the process could wait for it.
    * macOS x86-64: the SDK installs no-op handlers of `SIGPIPE` and `SIGXFSZ`, as on the other platforms.
    * The feature names `system::feature::MESSAGE_CONNECTORS` and `CONNECTION_ERRORS` are new.
* **\[DXFR-76]\[Rust]** `DXEndpoint::get_connectors` returns the message connectors of an endpoint. A
  `MessageConnector` gives its name, kind, masked address, current addresses, state, the active flag, the counts of
  open and closed connections and the bytes and data records read and written (`MessageConnectorStatistics`).
* **\[DXFR-77]\[Rust]** Experimental: with the `experimental` feature, `DXEndpoint::add_message_connector_listener`
  calls a closure on each change of a connector, and `MessageConnector::get_last_error` and
  `DXEndpoint::get_last_connection_error` return the last connection error (`ConnectionError`: time, masked address,
  `ConnectionErrorCategory`, the dropped flag, the class of the Java exception, the message and the count of errors).

### For integrators

* Depend on `dxfeed-graal = "0.2"`; `dxfeed-graal-sys` is `0.2.0+sdk.3.14.0`.
* A handler of `logging::set_handler` cannot take the levels `Trace` and `All` in SDK 3.14.0: use `Debug` or above
  with a handler.

## v0.1.1

* **\[DXFR-71]\[Rust]** `dxfeed-graal-sys`, `dxfeed-graal` and `dxfeed-graal-build` are published on crates.io, with
  their documentation on docs.rs. The version of `dxfeed-graal-sys` names the SDK in its build metadata
  (`0.1.1+sdk.3.13.0`).
* **\[DXFR-72]\[Rust]** The build script of `dxfeed-graal-sys` retries the SDK download after a timeout, a failed
  connection and an HTTP 429 or 5xx response, three attempts in all; other failures and a checksum mismatch still
  fail at once.

### For integrators

* Depend on `dxfeed-graal = "0.1"` from crates.io instead of a path or Git.

## v0.1.0

* The first release of the dxFeed Graal Rust API, on dxFeed Graal Native SDK v3.13.0.
    * Crates: `dxfeed-graal-sys` (raw bindings; its build script downloads, verifies and links the SDK),
      `dxfeed-graal` (the safe API) and `dxfeed-graal-build` (deploys the SDK library next to an application's
      executables). The crates are not published on crates.io yet; depend on them by path or by Git.
    * API: the dxFeed Java API 3.356 wherever the SDK exports it: endpoints of all roles, feeds, subscriptions
      (including time-series and indexed), listeners as closures, last events and promises, publishing, all event
      types (market, candle, option sale, order book, text and configuration messages, series, Greeks, ...), models
      (order book, indexed tx and time-series tx), instrument profiles and their collector and connection, option
      chains, schedules, on-demand replay, ORCS, system properties, logging and error handling. The experimental
      members of the SDK are behind the `experimental` feature. Events print as Java prints them.
    * Tools: one `Tools` binary with Connect, Dump, PerfTest, LatencyTest, Qds and Help, with the arguments, help
      screens and output of the C++ and .NET tools.
    * Samples: the samples of the Java API, with the names and groups of the C++ samples.
    * Releases: Tools and Samples archives for Linux x86-64 and arm64 (glibc 2.28 and newer), Windows x86-64 and macOS
      x86-64 and arm64, with SHA-256 checksums.
    * Platforms: Rust 1.88 or newer. CI checks Linux x86-64 and arm64, Windows x86-64 and macOS x86-64 and arm64.
