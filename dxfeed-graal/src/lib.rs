//! Safe Rust API for dxFeed market data over the dxFeed Graal Native SDK.
//!
//! The SDK runs in a GraalVM isolate that the crate creates on first use; threads are attached to it on their first
//! call and detached when they exit. Every fallible call returns [`Result`]; Java exceptions become [`Error::Java`].
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
