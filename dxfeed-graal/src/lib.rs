//! Safe Rust API for dxFeed market data over the dxFeed Graal Native SDK.
//!
//! The project is in alpha: the API may change in any release, and only a part of the SDK is covered.
//!
//! The names follow the dxFeed Java API: [`DXEndpoint`], [`DXFeed`], [`DXFeedSubscription`], [`DXPublisher`], with
//! methods in snake case (`addEventListener` is [`add_event_listener`](DXFeedSubscription::add_event_listener)) and
//! getters without `get` ([`DXEndpoint::feed`]); the Java names are search aliases in the documentation.
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
//!     let subscription = endpoint.feed().create_subscription::<Quote>()?;
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
//!     let quote = endpoint.feed().get_last_event_promise::<Quote>("AAPL")?.wait(Duration::from_secs(10))?;
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
//! # Threads
//!
//! The SDK runs in a GraalVM isolate that the crate creates on first use and never tears down. A thread is attached to
//! it on its first call and detached when it exits. All types are `Send` and `Sync`.
//!
//! # Listeners
//!
//! A listener's closure runs on a thread of the endpoint's executor, or inside
//! [`ManualExecutor::process_all_pending`]. The calls of one listener do not overlap. The events are valid during
//! the call only; copy what you keep with `to_owned()`. A panic in a closure is caught and printed to stderr.
//!
//! Add listeners before symbols. Removing a listener or dropping its [`DXFeedSubscription`] waits for its running
//! calls, except inside a callback, and so does dropping the last owner of an endpoint with state listeners. So do not
//! hold a lock that a closure takes while doing that from another thread.
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

mod endpoint;
mod error;
pub mod event;
mod executor;
mod feed;
mod handle;
mod isolate;
mod listener;
mod promise;
mod publisher;
mod subscription;
pub mod system;

pub use endpoint::{DXEndpoint, DXEndpointBuilder, Role, State};
pub use error::{Error, JavaException, Result};
#[doc(hidden)]
pub use event::sdk_event::live_events;
pub use event::{Event, EventKind, EventRef, EventType, Events, SubscriptionItem};
pub use executor::ManualExecutor;
pub use feed::DXFeed;
#[doc(hidden)]
pub use handle::live_handles;
pub use promise::Promise;
pub use publisher::DXPublisher;
pub use subscription::{DXFeedSubscription, ListenerId};

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
