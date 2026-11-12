# dxFeed Graal Rust API Release Notes

Each release has a section `## v<version>`; the release workflow publishes that section as the text of the GitHub
release (see [HOW_TO_RELEASE.md](HOW_TO_RELEASE.md)).

## v0.3.0

The release on dxFeed Graal Native SDK 3.16.0.

* **SDK.** The crates use SDK 3.16.0; the version of `dxfeed-graal-sys` names it (`0.3.0+sdk.3.16.0`). Depend on
  `dxfeed-graal = "0.3"`. The bindings change only in their doc comments.
* **Logging.** A handler of `logging::set_handler` works with `logging::set_log_level` at `Trace` and `All`: it gets
  the Java level `FINER` as `Trace` and `CONFIG` as `Debug`. Before, a call that logged at `FINER`, such as
  `DXEndpoint::connect`, failed with a `NullPointerException` while a handler was set.
* **Endpoint roles.** The SDK deprecates `dxfg_v4_DXEndpoint_getInstanceWithRole`, which creates a new endpoint on
  every call, and drops it in SDK 4.0.0. `DXEndpoint::get_instance_with_role` does not call it: it returns the
  default endpoint of the SDK for `Role::Feed` and keeps one endpoint per other role, created with
  `dxfg_v4_DXEndpoint_createWithRole`.
* **Properties files.** `dxfg_v4_DXEndpoint_Builder_withProperties` of the SDK now closes the file it reads.
  `DXEndpointBuilder::with_properties` of `dxfeed-graal` sets the properties one by one and does not use it.

## v0.2.1

The release on dxFeed Graal Native SDK 3.15.0; the API is the same as in 0.2.0.

* **SDK.** The crates use SDK 3.15.0, built with GraalVM 25.5 (JDK 25.0.5); the version of `dxfeed-graal-sys` names it
  (`0.2.1+sdk.3.15.0`). Its headers are those of SDK 3.14.0, so the bindings do not change.
* **TLS.** The SDK trusts the root CAs of JDK 25.0.5 unless `javax.net.ssl.trustStore` is set: it no longer trusts the
  eMudhra and Comodo roots that this JDK removes and rejects the blocked eMudhra TLS CAs G3B, so a connection to a
  server whose certificate chains up to one of them fails the TLS handshake. The macOS x86-64 library is still built
  with JDK 25.0.1, with the trust store of GraalVM 25.5 but without the distrust rules that later JDKs implement in
  code (the Chunghwa ePKI Root CA).
* **Logging.** A handler of `logging::set_handler` still cannot take the levels `Trace` and `All`.

## v0.2.0

The release on dxFeed Graal Native SDK 3.14.0.

* **SDK.** The crates use SDK 3.14.0; the version of `dxfeed-graal-sys` names it (`0.2.0+sdk.3.14.0`). Depend on
  `dxfeed-graal = "0.2"`.
* **Message connectors.** `DXEndpoint::get_connectors` returns the connectors of an endpoint. A `MessageConnector`
  gives its name, kind, masked address, current addresses, state, the active flag, the counts of open and closed
  connections and the bytes and data records read and written (`MessageConnectorStatistics`). The feature names
  `system::feature::MESSAGE_CONNECTORS` and `CONNECTION_ERRORS` are new.
* **Experimental: connector listener and connection errors.** With the `experimental` feature,
  `DXEndpoint::add_message_connector_listener` calls a closure on each change of a connector, and
  `MessageConnector::get_last_error` and `DXEndpoint::get_last_connection_error` return the last connection error
  (`ConnectionError`: time, masked address, `ConnectionErrorCategory`, the dropped flag, the class of the Java
  exception, the message and the count of errors).
* **Logging.** The SDK sets its root logger to the more verbose of the levels of `logging::set_log_level` (`INFO`
  by default) and `set_err_level`, so the messages below it are not created. A handler of `logging::set_handler`
  cannot take the levels `Trace` and `All` in SDK 3.14.0: use `Debug` or above with a handler.
* **Live instrument profiles.** Closing a live connection ends its thread with the next data or the read timeout;
  before, the end of the process could wait for it.
* **macOS x86-64.** The SDK installs no-op handlers of `SIGPIPE` and `SIGXFSZ` there, as on the other platforms.
* **Packaging.** The published manifests drop `license-file` next to the SPDX license, the packages of `dxfeed-graal`
  and `dxfeed-graal-sys` include their tests, and docs.rs documents all five supported targets.

## v0.1.1

The first release on crates.io, on dxFeed Graal Native SDK 3.13.0.

* **Crates.** `dxfeed-graal-sys`, `dxfeed-graal` and `dxfeed-graal-build` are published on crates.io, with their
  documentation on docs.rs: depend on `dxfeed-graal = "0.1"`. The version of `dxfeed-graal-sys` names the SDK in its
  build metadata (`0.1.1+sdk.3.13.0`).
* **SDK download.** The build script of `dxfeed-graal-sys` retries a timeout, a failed connection and an HTTP 429 or
  5xx response, three attempts in all; other failures and a checksum mismatch still fail at once.

## v0.1.0

The first release of the dxFeed Graal Rust API, on dxFeed Graal Native SDK 3.13.0.

* **Crates.** `dxfeed-graal-sys` (raw bindings; its build script downloads, verifies and links the SDK),
  `dxfeed-graal` (the safe API) and `dxfeed-graal-build` (deploys the SDK library next to an application's
  executables). The crates are not published on crates.io yet; depend on them by path or by Git.
* **API.** The dxFeed Java API 3.356 wherever the SDK exports it: endpoints of all roles, feeds, subscriptions
  (including time-series and indexed), listeners as closures, last events and promises, publishing, all event types
  (market, candle, option sale, order book, text and configuration messages, series, Greeks, ...), models (order
  book, indexed tx and time-series tx), instrument profiles and their collector and connection, option chains,
  schedules, on-demand replay, ORCS, system properties, logging and error handling. The experimental members of the
  SDK are behind the `experimental` feature. Events print as Java prints them.
* **Tools.** One `Tools` binary with Connect, Dump, PerfTest, LatencyTest, Qds and Help, with the arguments, help
  screens and output of the C++ and .NET tools.
* **Samples.** The samples of the Java API, with the names and groups of the C++ samples.
* **Releases.** Tools and Samples archives for Linux x86-64 and arm64 (glibc 2.28 and newer), Windows x86-64 and
  macOS x86-64 and arm64, with SHA-256 checksums.
* **Platforms.** Rust 1.88 or newer. CI checks Linux x86-64 and arm64, Windows x86-64 and macOS x86-64 and arm64.
