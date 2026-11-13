//! Safe Rust API for dxFeed market data over the dxFeed Graal Native SDK.
//!
//! The project is in alpha: the API may change in any release. It covers the dxFeed Java API 3.357 wherever the
//! SDK exports it; the README lists the Java members that wait for their wrappers or for new SDK exports.
//!
//! The names follow the dxFeed Java API: [`DXEndpoint`], [`DXFeed`], [`DXFeedSubscription`], [`DXPublisher`], with
//! methods in snake case: `addEventListener` is [`add_event_listener`](DXFeedSubscription::add_event_listener) and
//! `getFeed` is [`get_feed`](DXEndpoint::get_feed). The Java names are search aliases in the documentation.
//!
//! # Example
//!
//! Print the quotes of a symbol until Enter is pressed:
//!
//! ```no_run
//! use dxfeed_graal::DXEndpoint;
//! use dxfeed_graal::event::Quote;
//!
//! fn main() -> dxfeed_graal::Result<()> {
//!     let endpoint = DXEndpoint::create()?;
//!     let subscription = endpoint.get_feed().create_subscription::<Quote>()?;
//!     subscription.add_event_listener(|quotes| {
//!         for quote in quotes {
//!             println!("{quote}");
//!         }
//!     })?;
//!     subscription.add_symbols(["AAPL"])?;
//!     endpoint.connect("demo.dxfeed.com:7300")?;
//!     std::io::stdin().read_line(&mut String::new()).ok();
//!     endpoint.close_and_await_termination()
//! }
//! ```
//!
//! A quote prints as Java prints it: `Quote{AAPL, eventTime=0, time=20241004-093000.000, timeNanoPart=0, ...}`.
//! A subscription to several event types, [`DXFeed::create_subscription_with_types`], gives its listeners an
//! [`EventRef`] to match. All 27 event types of the SDK are mapped; see [`event`].
//!
//! The last event of a [`LastingEvent`](event::LastingEvent) type, such as a quote, comes from the local cache with
//! [`DXFeed::get_last_event_if_subscribed`] and [`DXFeed::get_last_event`], or from the uplink with
//! [`DXFeed::get_last_event_promise`], whose [`Promise`] is waited for with a timeout:
//!
//! ```no_run
//! use std::time::Duration;
//!
//! use dxfeed_graal::DXEndpoint;
//! use dxfeed_graal::event::Quote;
//!
//! fn main() -> dxfeed_graal::Result<()> {
//!     let endpoint = DXEndpoint::create()?;
//!     endpoint.connect("demo.dxfeed.com:7300")?;
//!     let quote = endpoint.get_feed().get_last_event_promise::<Quote>("AAPL")?.wait(Duration::from_secs(10))?;
//!     println!("{quote}");
//!     endpoint.close_and_await_termination()
//! }
//! ```
//!
//! The default feed of the process, [`DXFeed::get_instance`], connects to the address in the `dxfeed.address` system
//! property or in the `dxfeed.properties` file. System properties come from the code, from the
//! `dxfeed.system.properties` file and from the `DXFEED_*` environment variables (see [`system`]):
//!
//! ```no_run
//! use dxfeed_graal::event::Quote;
//! use dxfeed_graal::{DXFeed, system};
//!
//! fn main() -> dxfeed_graal::Result<()> {
//!     system::set_property("dxfeed.address", "demo.dxfeed.com:7300")?;
//!     let subscription = DXFeed::get_instance()?.create_subscription::<Quote>()?;
//!     subscription.add_event_listener(|quotes| quotes.for_each(|quote| println!("{quote}")))?;
//!     subscription.add_symbols(["AAPL"])?;
//!     std::io::stdin().read_line(&mut String::new()).ok();
//!     Ok(())
//! }
//! ```
//!
//! # How-to
//!
//! The common tasks have short examples on the items that do them; the `samples` directory of the repository has
//! complete programs.
//!
//! - Subscribe, add and change symbols, close; several event types at once: [`DXFeedSubscription`].
//! - Events from a time on, such as candles or time and sales: [`DXFeedTimeSeriesSubscription`]; a history in one
//!   request: [`DXFeed::get_time_series_promise`].
//! - The orders of one source, all the symbols, a time-series symbol: [`Symbol`].
//! - Candle symbols with a period, a price, an exchange or a session: [`CandleSymbol`](event::CandleSymbol).
//! - Read the fields of events, keep copies, use event flags: [`event`]; the main fields of
//!   [`Quote`](event::Quote), [`Trade`](event::Trade), [`TimeAndSale`](event::TimeAndSale), [`Order`](event::Order),
//!   [`Candle`](event::Candle) and [`Series`](event::Series).
//! - The last event of a symbol and other requests: [`Promise`], several at once: [`Promises`]; in async code: the
//!   [Async](#async) section.
//! - Conflate events or limit their batches: [`SubscriptionController`].
//! - Order books: [`OrderBookModel`](model::OrderBookModel), [`MarketDepthModel`](model::MarketDepthModel) and their
//!   lists, [`ObservableListModel`](model::ObservableListModel).
//! - Whole transactions and snapshots: [`IndexedTxModel`](model::IndexedTxModel),
//!   [`TimeSeriesTxModel`](model::TimeSeriesTxModel), [`IndexedEventTxModel`](model::IndexedEventTxModel); live
//!   lists: [`IndexedEventModel`](model::IndexedEventModel), [`TimeSeriesEventModel`](model::TimeSeriesEventModel).
//! - Publish events: [`DXPublisher`]; poll events without listeners: [`event_queue`].
//!
//! # Threads
//!
//! The SDK runs in a GraalVM isolate that the crate creates on first use and never tears down. A thread is attached to
//! it on its first call and detached when it exits. All types are `Send` and `Sync`.
//!
//! # Listeners
//!
//! A listener's closure runs on a thread of the endpoint's executor, its own threads by default or an [`Executor`]
//! thread pool, or inside [`ManualExecutor::process_all_pending`]. The calls of one listener do not overlap. The events
//! are valid during the call only; copy what you keep with `to_owned()`. A panic in a closure is caught and goes to
//! [`ErrorManager`].
//!
//! Without listeners, an [event queue](event_queue) gives the events of subscriptions to a thread of the application,
//! which polls them; the listeners of its subscriptions, and of the models and the collector on its executor, then run
//! inside the polls.
//!
//! Add listeners before symbols. Removing a listener or dropping its [`DXFeedSubscription`] waits for its running
//! calls, except inside a callback, and so does dropping the last owner of an endpoint with state listeners. So do not
//! hold a lock that a closure takes while doing that from another thread.
//!
//! # Async
//!
//! [`Promise`] and [`AnyOf`] implement [`IntoFuture`], so async code awaits them instead of
//! blocking a thread in [`wait`](Promise::wait). The futures work on any executor (tokio, async-std, smol or another);
//! the crate depends on no runtime and needs no cargo feature for them:
//!
//! ```no_run
//! use dxfeed_graal::event::Quote;
//! use dxfeed_graal::{DXFeed, Result};
//!
//! async fn last_quote(feed: &DXFeed) -> Result<Quote> {
//!     feed.get_last_event_promise::<Quote>("AAPL")?.await
//! }
//! ```
//!
//! A [`PromiseFuture`] keeps its promise: [`cancel`](PromiseFuture::cancel) makes it ready at once, so a timer of the
//! runtime that cancels it is the async timeout. Dropping the future does not cancel the request, closing the endpoint
//! leaves a pending request pending, and [`system::shutdown`] makes pending futures ready with [`Error::ShutDown`]. A
//! worker thread of the runtime that polls a future is attached to the isolate, as any thread that calls the SDK is.
//!
//! A subscription turns into an [`EventStream`] of owned events with
//! [`into_stream`](DXFeedSubscription::into_stream), before its symbols are added:
//!
//! ```no_run
//! use dxfeed_graal::DXFeed;
//! use dxfeed_graal::event::Quote;
//!
//! async fn print_quotes(feed: &DXFeed) -> dxfeed_graal::Result<()> {
//!     let mut quotes = feed.create_subscription::<Quote>()?.into_stream()?;
//!     quotes.subscription().add_symbols(["AAPL"])?;
//!     while let Some(quote) = quotes.next().await {
//!         println!("{quote}");
//!     }
//!     Ok(())
//! }
//! ```
//!
//! Its buffer drops the oldest events when it is full; with [`StreamOptions::event_queue`] the events stay in the SDK
//! until the consumer waits for them, and the SDK keeps the last quote of each symbol meanwhile ([`StreamOptions`]).
//! The stream ends when it or its subscription is closed, when its endpoints are closed and on [`system::shutdown`].
//! The cargo feature `stream` implements `futures_core::Stream` for it.
//!
//! The models stream whole items into a [`ModelStream`](model::ModelStream), built in place of their listener:
//! `build_stream` of the builders of [`IndexedTxModel`](model::IndexedTxModel) and
//! [`TimeSeriesTxModel`](model::TimeSeriesTxModel) yields a [`Transaction`](model::Transaction) per transaction or
//! snapshot, and `build_stream` of [`MarketDepthModel`](model::MarketDepthModel) or
//! [`OrderBookModel::into_stream`](model::OrderBookModel::into_stream) yields [`BookSides`](model::BookSides), the
//! whole book after a change. A model stream never splits an item: when its consumer lags, a snapshot drops the
//! buffered transactions of its source and a book replaces the buffered book, and otherwise the buffer grows. It ends
//! as an event stream does.
//!
//! # Errors
//!
//! Every fallible call returns [`Result`]. A Java exception becomes [`Error::Java`] with its class, message, stack
//! trace and causes; invalid arguments, such as strings with NUL bytes, are rejected before any SDK call. A wait that
//! times out returns [`Error::Timeout`].
//!
//! # Closing and exit
//!
//! Close an endpoint with [`DXEndpoint::close_and_await_termination`], which waits for the notifications already
//! scheduled, or with [`DXEndpoint::close`], which does not; dropping its last handle closes it as well. The process
//! may also exit with open endpoints and running listeners: from the moment it starts exiting, listeners are no longer
//! called and destructors no longer call the SDK.
//!
//! [`system::shutdown`] is an advanced function with no counterpart in the other dxFeed APIs: it stops all listeners
//! and closes all endpoints of the process, the default one included; afterwards nothing new can be created.
//!
//! # Deployment
//!
//! The SDK library is linked dynamically and must be found at run time; see the README of the repository.
#![warn(missing_docs)]

mod auth_token;
mod connector;
mod endpoint;
mod error;
mod error_manager;
pub mod event;
pub mod event_queue;
mod executor;
mod feed;
#[cfg(feature = "experimental")]
pub mod feed_filter;
pub mod glossary;
mod handle;
pub mod history;
pub mod ipf;
mod isolate;
mod listener;
pub mod logging;
pub mod model;
mod observable;
pub mod ondemand;
pub mod orcs;
mod promise;
mod publisher;
pub mod qds;
pub mod schedule;
mod stream;
mod subscription;
mod subscription_controller;
mod symbol;
pub mod system;
mod terminate;
mod time_format;
mod time_period;
mod time_zone;

pub use auth_token::AuthToken;
#[cfg(feature = "experimental")]
pub use connector::{ConnectionError, ConnectionErrorCategory};
pub use connector::{MessageConnector, MessageConnectorKind, MessageConnectorState, MessageConnectorStatistics};
pub use endpoint::{DXEndpoint, DXEndpointBuilder, Role, State};
pub use error::{Error, JavaException, JavaExceptionClass, Result};
pub use error_manager::{ErrorInfo, ErrorManager};
#[doc(hidden)]
pub use event::sdk_event::live_events;
pub use event::{Event, EventKind, EventRef, EventType, Events, SubscriptionItem};
pub use executor::{Executor, ManualExecutor};
pub use feed::DXFeed;
#[doc(hidden)]
pub use handle::live_handles;
pub use observable::{Observable, ObservableSubscription, ObservableSubscriptionChangeListener};
pub use promise::{AnyOf, AnyOfFuture, Promise, PromiseFuture, PromiseResult, Promises};
pub use publisher::DXPublisher;
pub use stream::{EventStream, StreamOptions};
pub use subscription::{DXFeedSubscription, DXFeedTimeSeriesSubscription, ListenerId};
pub use subscription_controller::SubscriptionController;
pub use symbol::Symbol;
pub use time_format::TimeFormat;
pub use time_period::{TimePeriod, TimePeriodInfo};
pub use time_zone::TimeZone;

/// Locks `mutex`, ignoring poisoning: the data it guards stays consistent when a holder panics.
fn lock<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Returns the value of `cell`, setting it to the result of `init` under `guard` if it is empty. A failed `init` leaves
/// the cell empty, so that the next call tries again.
fn get_or_try_init<'a, T, E>(
    cell: &'a std::sync::OnceLock<T>,
    guard: &std::sync::Mutex<()>,
    init: impl FnOnce() -> std::result::Result<T, E>,
) -> std::result::Result<&'a T, E> {
    if let Some(value) = cell.get() {
        return Ok(value);
    }
    let _guard = lock(guard);
    if let Some(value) = cell.get() {
        return Ok(value);
    }
    let value = init()?;
    Ok(cell.get_or_init(|| value))
}
