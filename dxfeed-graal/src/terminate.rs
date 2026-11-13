//! What [`system::shutdown`](crate::system::shutdown) and the close of an endpoint end besides the SDK: the pending
//! futures of promises and the event streams.

use std::panic::{self, AssertUnwindSafe};
use std::sync::{Arc, Mutex, Weak};

use crate::endpoint::EndpointState;

/// Something that [`system::shutdown`](crate::system::shutdown) ends.
pub(crate) trait Terminate: Send + Sync {
    /// Ends it; does nothing if it has ended.
    fn terminate(&self);

    /// Ends it if it depends on `endpoint`, which has just been closed; does nothing by default.
    fn endpoint_closed(&self, endpoint: &EndpointState) {
        let _ = endpoint;
    }

    /// Prepares it for the wait of [`close_and_await_termination`](crate::DXEndpoint::close_and_await_termination) of
    /// `endpoint`, which is marked closed and is about to wait for its scheduled work; does nothing by default.
    fn endpoint_closing(&self, endpoint: &EndpointState) {
        let _ = endpoint;
    }
}

/// The live entries, and whether shutdown has begun.
struct Registry {
    shut_down: bool,
    entries: Vec<Weak<dyn Terminate>>,
    /// The length at which the next insertion drops the dead entries first.
    prune_at: usize,
}

const MIN_PRUNE_AT: usize = 64;

static REGISTRY: Mutex<Registry> = Mutex::new(Registry {
    shut_down: false,
    entries: Vec::new(),
    prune_at: MIN_PRUNE_AT,
});

/// Registers `entry` for shutdown, or ends it at once if shutdown has begun. The registry keeps it weakly.
pub(crate) fn register<T: Terminate + 'static>(entry: &Arc<T>) {
    {
        let mut registry = crate::lock(&REGISTRY);
        if !registry.shut_down {
            if registry.entries.len() >= registry.prune_at {
                registry.entries.retain(|entry| entry.strong_count() > 0);
                registry.prune_at = (registry.entries.len() * 2).max(MIN_PRUNE_AT);
            }
            let entry: Weak<T> = Arc::downgrade(entry);
            registry.entries.push(entry);
            return;
        }
    }
    entry.terminate();
}

/// Ends the registered entries and every one registered later. The entries are ended outside the lock; a panic of
/// one, such as of the waker of a future, goes to [`ErrorManager`](crate::ErrorManager) and the others are still ended.
pub(crate) fn terminate_all() {
    let entries = {
        let mut registry = crate::lock(&REGISTRY);
        registry.shut_down = true;
        std::mem::take(&mut registry.entries)
    };
    for entry in entries.iter().filter_map(Weak::upgrade) {
        crate::error_manager::report_panic(
            "System",
            "a waker at shutdown",
            panic::catch_unwind(AssertUnwindSafe(|| entry.terminate())),
        );
    }
}

/// Tells the registered entries that `endpoint` has been closed, outside the lock; a panic of one goes to
/// [`ErrorManager`](crate::ErrorManager) and the others are still told.
pub(crate) fn endpoint_closed(endpoint: &EndpointState) {
    for_each_entry(|entry| {
        crate::error_manager::report_panic(
            "DXEndpoint",
            "a waker at close",
            panic::catch_unwind(AssertUnwindSafe(|| entry.endpoint_closed(endpoint))),
        );
    });
}

/// Tells the registered entries that `endpoint` is about to wait for its termination, outside the lock; a panic of one
/// goes to [`ErrorManager`](crate::ErrorManager) and the others are still told.
pub(crate) fn endpoint_closing(endpoint: &EndpointState) {
    for_each_entry(|entry| {
        crate::error_manager::report_panic(
            "DXEndpoint",
            "the close of an event stream",
            panic::catch_unwind(AssertUnwindSafe(|| entry.endpoint_closing(endpoint))),
        );
    });
}

/// Calls `f` with each live entry, outside the lock.
fn for_each_entry(mut f: impl FnMut(&dyn Terminate)) {
    let entries = crate::lock(&REGISTRY).entries.clone();
    for entry in entries.iter().filter_map(Weak::upgrade) {
        f(&*entry);
    }
}
