//! Subscriptions to events.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

use dxfeed_graal_sys as sys;

use crate::error::{Error, Result, c_string};
use crate::event::Events;
use crate::handle::Handle;
use crate::isolate::{self, call};
use crate::listener::Listener;

/// Identifies a listener added by [`Subscription::add_listener`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ListenerId(u64);

/// A subscription of a [`Feed`](crate::Feed) to events of some types for some symbols (`DXFeedSubscription`).
///
/// Dropping it removes and releases its listeners, then closes it. A listener's closure is not called after the
/// listener is released; dropping the subscription from another thread therefore waits for running calls of its
/// listeners, so do not hold a lock their closures take while dropping it.
pub struct Subscription {
    handle: Handle<sys::dxfg_v4_subscription_t>,
    listeners: Mutex<Vec<(ListenerId, Listener)>>,
}

impl Subscription {
    pub(crate) fn new(handle: Handle<sys::dxfg_v4_subscription_t>) -> Self {
        Subscription {
            handle,
            listeners: Mutex::new(Vec::new()),
        }
    }

    /// Adds a listener; `f` is called with each batch of received events, on a thread of the endpoint's executor.
    ///
    /// Add listeners before symbols, so the first events are not missed. A panic in `f` is caught and printed to
    /// stderr; the listener stays active. The events are valid during the call only: copy what you keep with
    /// `to_owned()`.
    pub fn add_listener(&self, f: impl FnMut(Events<'_>) + Send + 'static) -> Result<ListenerId> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let listener = Listener::new(Box::new(f))?;
        let (s, l) = (self.handle.as_ptr(), listener.as_ptr());
        // SAFETY: the subscription and the listener are live.
        if let Err(e) = call(|t| unsafe { sys::dxfg_v4_DXFeedSubscription_addEventListener(t, s, l) }) {
            listener.release();
            return Err(e);
        }
        let id = ListenerId(NEXT.fetch_add(1, Ordering::Relaxed));
        self.lock().push((id, listener));
        Ok(id)
    }

    /// Removes a listener. Called outside listeners, it returns once no call of the listener runs; called from a
    /// listener, it returns at once and the listener is released in the background.
    ///
    /// Returns [`Error::InvalidArgument`] if the listener is not in this subscription.
    pub fn remove_listener(&self, id: ListenerId) -> Result<()> {
        let listener = {
            let mut listeners = self.lock();
            let index = listeners
                .iter()
                .position(|(i, _)| *i == id)
                .ok_or_else(|| Error::InvalidArgument(format!("{id:?} is not a listener of this subscription")))?;
            listeners.swap_remove(index).1
        };
        self.remove(listener)
    }

    /// Adds symbols, e.g. `["AAPL", "IBM"]`.
    pub fn add_symbols<S: AsRef<str>>(&self, symbols: &[S]) -> Result<()> {
        let s = self.handle.as_ptr();
        with_symbols(symbols, |list| {
            // SAFETY: the subscription and the list are live.
            call(|t| unsafe { sys::dxfg_v4_DXFeedSubscription_addSymbols(t, s, list) })
        })
    }

    /// Removes symbols.
    pub fn remove_symbols<S: AsRef<str>>(&self, symbols: &[S]) -> Result<()> {
        let s = self.handle.as_ptr();
        with_symbols(symbols, |list| {
            // SAFETY: the subscription and the list are live.
            call(|t| unsafe { sys::dxfg_v4_DXFeedSubscription_removeSymbols(t, s, list) })
        })
    }

    /// Closes the subscription: it detaches from the feed and its listeners get no new events, except possibly one
    /// batch already under way. Closing again does nothing.
    pub fn close(&self) -> Result<()> {
        let s = self.handle.as_ptr();
        // SAFETY: the subscription is live.
        call(|t| unsafe { sys::dxfg_v4_DXFeedSubscription_close(t, s) })
    }

    fn lock(&self) -> MutexGuard<'_, Vec<(ListenerId, Listener)>> {
        crate::lock(&self.listeners)
    }

    fn remove(&self, listener: Listener) -> Result<()> {
        let (s, l) = (self.handle.as_ptr(), listener.as_ptr());
        // SAFETY: the subscription and the listener are live.
        let removed = call(|t| unsafe { sys::dxfg_v4_DXFeedSubscription_removeEventListener(t, s, l) });
        listener.release();
        removed
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if isolate::exiting() {
            // The process is exiting: no SDK calls from destructors. The listeners are dropped without being released,
            // which leaks their state, as the SDK may still call their trampoline.
            return;
        }
        let listeners = std::mem::take(&mut *self.lock());
        for (_, listener) in listeners {
            let _ = self.remove(listener);
        }
        let _ = self.close();
    }
}

/// Calls `f` with an SDK list of string symbols.
fn with_symbols<S: AsRef<str>>(
    symbols: &[S],
    f: impl FnOnce(*mut sys::dxfg_v4_symbol_list) -> Result<()>,
) -> Result<()> {
    let strings = symbols
        .iter()
        .map(|s| c_string(s.as_ref(), "symbol"))
        .collect::<Result<Vec<_>>>()?;
    let mut structs: Vec<sys::dxfg_string_symbol_t> = strings
        .iter()
        .map(|s| sys::dxfg_string_symbol_t {
            supper: sys::dxfg_symbol_t { type_: sys::STRING },
            symbol: s.as_ptr(),
        })
        .collect();
    let mut pointers: Vec<*mut sys::dxfg_symbol_t> = structs
        .iter_mut()
        .map(|s| (s as *mut sys::dxfg_string_symbol_t).cast())
        .collect();
    let mut list = sys::dxfg_symbol_list {
        size: pointers.len() as i32,
        elements: pointers.as_mut_ptr(),
    };
    f(&mut list)
}
