//! Safe Rust API for dxFeed market data over the dxFeed Graal Native SDK.
//!
//! The project is in alpha: the API may change in any release, and only a part of the SDK is covered.
//!
//! # Example
//!
//! ```no_run
//! use dxfeed_graal::{Endpoint, EventRef, EventType, Role};
//!
//! fn main() -> dxfeed_graal::Result<()> {
//!     let endpoint = Endpoint::create(Role::Feed)?;
//!     endpoint.connect("demo.dxfeed.com:7300")?;
//!     let subscription = endpoint.feed()?.create_subscription(&[EventType::Quote])?;
//!     subscription.add_listener(|events| {
//!         for event in events {
//!             if let EventRef::Quote(quote) = event {
//!                 println!("{quote:?}");
//!             }
//!         }
//!     })?;
//!     subscription.add_symbols(&["AAPL"])?;
//!     std::thread::sleep(std::time::Duration::from_secs(10));
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
//! Add listeners before symbols. Removing a listener or dropping its [`Subscription`] waits for its running calls,
//! except inside a callback, so do not hold a lock that a closure takes while doing that from another thread.
//!
//! # Errors
//!
//! Every fallible call returns [`Result`]. A Java exception becomes [`Error::Java`] with its class, message, stack
//! trace and causes; invalid arguments, such as strings with NUL bytes, are rejected before any SDK call.
//!
//! # Shutdown and exit
//!
//! The process may exit with open endpoints and running listeners: from the moment it starts exiting, listeners are no
//! longer called and destructors no longer call the SDK. [`system::shutdown`] stops the listeners and closes all
//! endpoints explicitly; afterwards nothing new can be created.
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
mod publisher;
mod subscription;
pub mod system;

pub use endpoint::{Endpoint, EndpointBuilder, Role};
pub use error::{Error, JavaException, Result};
pub use event::{Event, EventRef, EventType, Events};
pub use executor::ManualExecutor;
pub use feed::Feed;
#[doc(hidden)]
pub use handle::live_handles;
pub use publisher::Publisher;
pub use subscription::{ListenerId, Subscription};

/// Locks `mutex`, ignoring poisoning: the data it guards stays consistent when a holder panics.
fn lock<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}
