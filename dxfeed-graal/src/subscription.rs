//! Subscriptions to events.

use std::marker::PhantomData;
use std::ops::Deref;
use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

use dxfeed_graal_sys as sys;

use crate::error::{Error, Result};
use crate::event::{Event, EventKind, EventType, Events, SubscriptionItem, TimeSeriesEvent};
use crate::feed::DXFeed;
use crate::handle::Handle;
use crate::isolate::{self, call};
use crate::listener::{ChangeListener, ChangeListeners, EventListener};
use crate::observable::{Observable, ObservableSubscriptionChangeListener, event_types};
use crate::symbol::{Symbol, SymbolArena};

/// Identifies a listener added by [`DXFeedSubscription::add_event_listener`], an `add_change_listener` method or
/// [`DXEndpoint::add_state_listener`](crate::DXEndpoint::add_state_listener).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ListenerId(u64);

impl ListenerId {
    /// A new id, unique in the process.
    pub(crate) fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        ListenerId(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

/// A subscription of a [`DXFeed`] to events for some symbols (`DXFeedSubscription`).
///
/// `E` is what its listeners receive: an event type such as [`Quote`](crate::event::Quote) for a subscription created
/// with [`DXFeed::create_subscription`], [`Event`] for one created with [`DXFeed::create_subscription_with_types`].
///
/// Dropping it removes and releases its event and change listeners, then closes it. A listener's closure is not called after the
/// listener is released; dropping the subscription from another thread therefore waits for running calls of its
/// listeners, so do not hold a lock their closures take while dropping it. It keeps its feed's endpoint open.
pub struct DXFeedSubscription<E: SubscriptionItem = Event> {
    handle: Handle<sys::dxfg_v4_subscription_t>,
    listeners: Mutex<Vec<(ListenerId, EventListener)>>,
    change_listeners: ChangeListeners,
    // Dropped last: it may close the endpoint.
    _feed: DXFeed,
    _item: PhantomData<fn() -> E>,
}

impl<E: SubscriptionItem> DXFeedSubscription<E> {
    pub(crate) fn new(handle: Handle<sys::dxfg_v4_subscription_t>, feed: DXFeed) -> Self {
        DXFeedSubscription {
            handle,
            listeners: Mutex::new(Vec::new()),
            change_listeners: ChangeListeners::new("a change listener of this subscription"),
            _feed: feed,
            _item: PhantomData,
        }
    }

    /// Adds a listener; `f` is called with each batch of received events, on a thread of the endpoint's executor.
    ///
    /// The batch iterates `&QuoteRef` for a subscription to quotes, `&TradeRef` for one to trades and
    /// [`EventRef`](crate::EventRef) for an untyped one. Add listeners before symbols, so the first events are not
    /// missed. A panic in `f` is caught and goes to [`ErrorManager`](crate::ErrorManager); the listener stays active.
    /// The events are valid during the call only: copy what you keep with `to_owned()`.
    #[doc(alias = "addEventListener")]
    pub fn add_event_listener(&self, mut f: impl FnMut(Events<'_, E>) + Send + 'static) -> Result<ListenerId> {
        let listener = EventListener::new(Box::new(move |events: Events<'_>| f(events.typed())))?;
        let (s, l) = (self.handle.as_ptr(), listener.as_ptr());
        // SAFETY: the subscription and the listener are live.
        if let Err(e) = call(|t| unsafe { sys::dxfg_v4_DXFeedSubscription_addEventListener(t, s, l) }) {
            listener.release();
            return Err(e);
        }
        let id = ListenerId::next();
        self.lock().push((id, listener));
        Ok(id)
    }

    /// Removes a listener. Called outside listeners, it returns once no call of the listener runs; called from a
    /// listener, it returns at once and the listener is released in the background.
    ///
    /// Returns [`Error::InvalidArgument`] if the listener is not in this subscription.
    #[doc(alias = "removeEventListener")]
    pub fn remove_event_listener(&self, id: ListenerId) -> Result<()> {
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

    /// Adds a change listener; before this returns, the listener is called with the symbols the subscription has
    /// (`addChangeListener`). Adding to a closed subscription does nothing.
    #[doc(alias = "addChangeListener")]
    pub fn add_change_listener(&self, listener: impl ObservableSubscriptionChangeListener) -> Result<ListenerId> {
        let s = self.handle.as_ptr();
        self.change_listeners
            .add(ChangeListener::new(Box::new(listener))?, |l| {
                // SAFETY: the subscription and the listener are live.
                call(|t| unsafe { sys::dxfg_v4_DXFeedSubscription_addChangeListener(t, s, l) })
            })
    }

    /// Removes a change listener, which then gets its
    /// [`subscription_closed`](ObservableSubscriptionChangeListener::subscription_closed) call
    /// (`removeChangeListener`). Called outside the listener, it returns once no call of the listener runs; called
    /// from the listener, it returns at once and the listener is released in the background.
    ///
    /// Returns [`Error::InvalidArgument`] if the listener is not in this subscription.
    #[doc(alias = "removeChangeListener")]
    pub fn remove_change_listener(&self, id: ListenerId) -> Result<()> {
        self.change_listeners.remove(id, |l| self.remove_change(l))
    }

    /// Adds symbols: any iterable of strings or [`Symbol`]s, e.g. `["AAPL", "IBM"]`, a `Vec<String>`,
    /// `std::iter::once("AAPL")` or `[Symbol::Wildcard]`.
    #[doc(alias = "addSymbols")]
    pub fn add_symbols(&self, symbols: impl IntoIterator<Item: Into<Symbol>>) -> Result<()> {
        let mut arena = SymbolArena::new();
        let list = arena.list(symbols)?;
        let s = self.handle.as_ptr();
        // SAFETY: the subscription and the list are live; the SDK only reads the list.
        call(|t| unsafe { sys::dxfg_v4_DXFeedSubscription_addSymbols(t, s, list) })
    }

    /// Removes symbols; takes the same iterables as [`add_symbols`](Self::add_symbols).
    #[doc(alias = "removeSymbols")]
    pub fn remove_symbols(&self, symbols: impl IntoIterator<Item: Into<Symbol>>) -> Result<()> {
        let mut arena = SymbolArena::new();
        let list = arena.list(symbols)?;
        let s = self.handle.as_ptr();
        // SAFETY: the subscription and the list are live; the SDK only reads the list.
        call(|t| unsafe { sys::dxfg_v4_DXFeedSubscription_removeSymbols(t, s, list) })
    }

    /// Replaces the symbols with `symbols`, which takes the same iterables as [`add_symbols`](Self::add_symbols)
    /// (`setSymbols`). The change listeners see the symbols added and removed before this returns.
    #[doc(alias = "setSymbols")]
    pub fn set_symbols(&self, symbols: impl IntoIterator<Item: Into<Symbol>>) -> Result<()> {
        let mut arena = SymbolArena::new();
        let list = arena.list(symbols)?;
        let s = self.handle.as_ptr();
        // SAFETY: the subscription and the list are live; the SDK only reads the list.
        call(|t| unsafe { sys::dxfg_v4_DXFeedSubscription_setSymbols(t, s, list) })
    }

    /// Removes all symbols (`clear`).
    pub fn clear(&self) -> Result<()> {
        let s = self.handle.as_ptr();
        // SAFETY: the subscription is live.
        call(|t| unsafe { sys::dxfg_v4_DXFeedSubscription_clear(t, s) })
    }

    /// The symbols as they were added, in no particular order (`getSymbols`). Those of a
    /// [`DXFeedTimeSeriesSubscription`] come without the time of the subscription.
    #[doc(alias = "getSymbols")]
    pub fn get_symbols(&self) -> Result<Vec<Symbol>> {
        let s = self.handle.as_ptr();
        // SAFETY: the subscription is live and the out-pointer is valid for writes.
        self.symbols(|t, list| unsafe { sys::dxfg_v4_DXFeedSubscription_getSymbols(t, s, list) })
    }

    /// The symbols as the change listeners see them, in no particular order (`getDecoratedSymbols`): those of a
    /// [`DXFeedTimeSeriesSubscription`] are [`Symbol::TimeSeriesSubscription`]s with the time of the subscription,
    /// those of other subscriptions are the symbols of [`get_symbols`](Self::get_symbols).
    #[doc(alias = "getDecoratedSymbols")]
    pub fn get_decorated_symbols(&self) -> Result<Vec<Symbol>> {
        let s = self.handle.as_ptr();
        // SAFETY: the subscription is live and the out-pointer is valid for writes.
        self.symbols(|t, list| unsafe { sys::dxfg_v4_DXFeedSubscription_getDecoratedSymbols(t, s, list) })
    }

    /// Whether the subscription is closed (`isClosed`).
    #[doc(alias = "isClosed")]
    pub fn is_closed(&self) -> Result<bool> {
        let (s, mut closed) = (self.handle.as_ptr(), 0);
        // SAFETY: the subscription is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_DXFeedSubscription_isClosed(t, s, &mut closed) })?;
        Ok(closed != 0)
    }

    /// The event types of the subscription (`getEventTypes`).
    #[doc(alias = "getEventTypes")]
    pub fn get_event_types(&self) -> Result<Vec<EventType>> {
        let (s, mut list) = (self.handle.as_ptr(), ptr::null_mut());
        // SAFETY: the subscription is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_DXFeedSubscription_getEventTypes(t, s, &mut list) })?;
        // SAFETY: the SDK returned a live list of `size` live classes, or NULL.
        let types = unsafe { event_types(list) };
        // SAFETY: the list came from the SDK and is released once.
        let released = call(|t| unsafe { sys::dxfg_v4_event_clazz_list_release(t, list) });
        crate::error_manager::report_release("DXFeedSubscription", released);
        Ok(types)
    }

    /// Whether the subscription is to the events of `event_type` (`containsEventType`).
    #[doc(alias = "containsEventType")]
    pub fn contains_event_type(&self, event_type: EventType) -> Result<bool> {
        let (s, mut contains) = (self.handle.as_ptr(), 0);
        // SAFETY: the subscription is live and the out-pointer is valid for writes.
        call(|t| unsafe {
            sys::dxfg_v4_DXFeedSubscription_containsEventType(t, s, event_type.to_sys(), &mut contains)
        })?;
        Ok(contains != 0)
    }

    /// Closes the subscription: it detaches from the feed and its listeners get no new events, except possibly one
    /// batch already under way. Closing again does nothing.
    pub fn close(&self) -> Result<()> {
        let s = self.handle.as_ptr();
        // SAFETY: the subscription is live.
        call(|t| unsafe { sys::dxfg_v4_DXFeedSubscription_close(t, s) })
    }

    /// The symbols of a list that `f` gets from the SDK, which is released after reading.
    fn symbols(
        &self,
        f: impl FnOnce(isolate::Thread, *mut *mut sys::dxfg_v4_symbol_list) -> i32,
    ) -> Result<Vec<Symbol>> {
        let mut list = ptr::null_mut();
        call(|t| f(t, &mut list))?;
        // SAFETY: the SDK returned a live list of live symbols, or NULL.
        let symbols = unsafe { Symbol::list_from_sys(list, "DXFeedSubscription") };
        if !list.is_null() {
            // SAFETY: the list came from the SDK and is released once.
            let released = call(|t| unsafe { sys::dxfg_v4_symbol_list_release(t, list) });
            crate::error_manager::report_release("DXFeedSubscription", released);
        }
        Ok(symbols)
    }

    fn lock(&self) -> MutexGuard<'_, Vec<(ListenerId, EventListener)>> {
        crate::lock(&self.listeners)
    }

    fn remove_change(&self, listener: *mut sys::dxfg_v4_observable_subscription_change_listener_t) -> Result<()> {
        let s = self.handle.as_ptr();
        // SAFETY: the subscription and the listener are live.
        call(|t| unsafe { sys::dxfg_v4_DXFeedSubscription_removeChangeListener(t, s, listener) })
    }

    fn remove(&self, listener: EventListener) -> Result<()> {
        let (s, l) = (self.handle.as_ptr(), listener.as_ptr());
        // SAFETY: the subscription and the listener are live.
        let removed = call(|t| unsafe { sys::dxfg_v4_DXFeedSubscription_removeEventListener(t, s, l) });
        listener.release();
        removed
    }
}

impl<E: SubscriptionItem> crate::observable::sealed::Sealed for DXFeedSubscription<E> {}

impl<E: SubscriptionItem> Observable for DXFeedSubscription<E> {
    fn is_closed(&self) -> Result<bool> {
        DXFeedSubscription::is_closed(self)
    }

    fn get_event_types(&self) -> Result<Vec<EventType>> {
        DXFeedSubscription::get_event_types(self)
    }

    fn contains_event_type(&self, event_type: EventType) -> Result<bool> {
        DXFeedSubscription::contains_event_type(self, event_type)
    }

    fn add_change_listener(&self, listener: impl ObservableSubscriptionChangeListener) -> Result<ListenerId> {
        DXFeedSubscription::add_change_listener(self, listener)
    }

    fn remove_change_listener(&self, id: ListenerId) -> Result<()> {
        DXFeedSubscription::remove_change_listener(self, id)
    }
}

impl<E: SubscriptionItem> Drop for DXFeedSubscription<E> {
    fn drop(&mut self) {
        if isolate::exiting() {
            // The process is exiting: no SDK calls from destructors. The listeners are dropped without being released,
            // which leaks their state, as the SDK may still call their trampoline.
            return;
        }
        let listeners = std::mem::take(&mut *self.lock());
        for (_, listener) in listeners {
            crate::error_manager::report_release("DXFeedSubscription", self.remove(listener));
        }
        self.change_listeners
            .remove_all("DXFeedSubscription", |l| self.remove_change(l));
        crate::error_manager::report_release("DXFeedSubscription", self.close());
    }
}

/// A subscription to the time-series events of type `E` from a time on (`DXFeedTimeSeriesSubscription`), created with
/// [`DXFeed::create_time_series_subscription`].
///
/// It is a [`DXFeedSubscription`], whose methods it has through `Deref`, that wraps each symbol it adds in a
/// [`Symbol::TimeSeriesSubscription`] from the time of [`set_from_time`](Self::set_from_time).
pub struct DXFeedTimeSeriesSubscription<E: TimeSeriesEvent + EventKind> {
    subscription: DXFeedSubscription<E>,
}

impl<E: TimeSeriesEvent + EventKind> DXFeedTimeSeriesSubscription<E> {
    pub(crate) fn new(subscription: DXFeedSubscription<E>) -> Self {
        DXFeedTimeSeriesSubscription { subscription }
    }

    /// Sets the time, in milliseconds since the Unix epoch, of the first events of all its symbols, those added before
    /// and after (`setFromTime`).
    #[doc(alias = "setFromTime")]
    pub fn set_from_time(&self, from_time: i64) -> Result<()> {
        let s = self
            .subscription
            .handle
            .as_ptr()
            .cast::<sys::dxfg_v4_time_series_subscription_t>();
        // SAFETY: the subscription was created as a time-series subscription and is live.
        call(|t| unsafe { sys::dxfg_v4_DXFeedTimeSeriesSubscription_setFromTime(t, s, from_time) })
    }
}

impl<E: TimeSeriesEvent + EventKind> crate::observable::sealed::Sealed for DXFeedTimeSeriesSubscription<E> {}

impl<E: TimeSeriesEvent + EventKind> Observable for DXFeedTimeSeriesSubscription<E> {
    fn is_closed(&self) -> Result<bool> {
        self.subscription.is_closed()
    }

    fn get_event_types(&self) -> Result<Vec<EventType>> {
        self.subscription.get_event_types()
    }

    fn contains_event_type(&self, event_type: EventType) -> Result<bool> {
        self.subscription.contains_event_type(event_type)
    }

    fn add_change_listener(&self, listener: impl ObservableSubscriptionChangeListener) -> Result<ListenerId> {
        self.subscription.add_change_listener(listener)
    }

    fn remove_change_listener(&self, id: ListenerId) -> Result<()> {
        self.subscription.remove_change_listener(id)
    }
}

impl<E: TimeSeriesEvent + EventKind> Deref for DXFeedTimeSeriesSubscription<E> {
    type Target = DXFeedSubscription<E>;

    fn deref(&self) -> &DXFeedSubscription<E> {
        &self.subscription
    }
}
