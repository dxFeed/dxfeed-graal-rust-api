//! Subscriptions to events.

use std::marker::PhantomData;
use std::ops::Deref;
use std::ptr;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

use dxfeed_graal_sys as sys;

use crate::error::{Error, Result};
use crate::event::{Event, EventKind, EventType, Events, SubscriptionItem};
use crate::executor::Executor;
use crate::feed::DXFeed;
use crate::handle::Handle;
use crate::isolate::{self, call};
use crate::listener::{ChangeListener, ChangeListeners, EventListener};
use crate::observable::{Observable, ObservableSubscriptionChangeListener, event_types};
use crate::subscription_controller::SubscriptionController;
use crate::symbol::{Symbol, SymbolArena};
use crate::time_period::{TimePeriod, TimePeriodInfo};

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
/// with [`DXFeed::create_subscription`] or [`new`](Self::new), [`Event`] for one created with
/// [`DXFeed::create_subscription_with_types`] or [`new_with_types`](DXFeedSubscription::new_with_types). One created
/// without a feed receives events once it is [attached](Self::attach) to one.
///
/// Dropping it removes and releases its event and change listeners, then closes it. A listener's closure is not called after the
/// listener is released; dropping the subscription from another thread therefore waits for running calls of its
/// listeners, so do not hold a lock their closures take while dropping it. It keeps the endpoints of the feeds it is
/// attached to open.
pub struct DXFeedSubscription<E: SubscriptionItem = Event> {
    handle: Handle<sys::dxfg_v4_subscription_t>,
    listeners: Mutex<Vec<(ListenerId, EventListener)>>,
    change_listeners: ChangeListeners,
    /// The feeds it is attached to; dropped last, as they may close their endpoints.
    feeds: Mutex<Vec<DXFeed>>,
    _item: PhantomData<fn() -> E>,
}

impl<E: EventKind> DXFeedSubscription<E> {
    /// Creates a subscription to events of type `E` that is attached to no feed (`new DXFeedSubscription(Class)`):
    /// attach it with [`attach`](Self::attach) or [`DXFeed::attach_subscription`].
    pub fn new() -> Result<Self> {
        // SAFETY: the out-pointer is valid for writes.
        let handle = Handle::create(|t, out| unsafe { sys::dxfg_v4_DXFeedSubscription_new(t, E::TYPE.to_sys(), out) })?;
        Ok(DXFeedSubscription::from_sdk(handle, Vec::new()))
    }
}

impl DXFeedSubscription<Event> {
    /// Creates a subscription to events of `types` that is attached to no feed; its listeners receive
    /// [`EventRef`](crate::EventRef) (`new DXFeedSubscription(Class...)`).
    ///
    /// Returns [`Error::InvalidArgument`] if `types` is empty.
    pub fn new_with_types(types: &[EventType]) -> Result<Self> {
        let mut classes = ClassList::new(types)?;
        let list = classes.as_sys();
        // SAFETY: the list is live and the SDK only reads it; the out-pointer is valid for writes.
        let handle =
            Handle::create(|t, out| unsafe { sys::dxfg_v4_DXFeedSubscription_newWithEventTypes(t, list, out) })?;
        Ok(DXFeedSubscription::from_sdk(handle, Vec::new()))
    }
}

impl<E: SubscriptionItem> DXFeedSubscription<E> {
    pub(crate) fn from_sdk(handle: Handle<sys::dxfg_v4_subscription_t>, feeds: Vec<DXFeed>) -> Self {
        DXFeedSubscription {
            handle,
            listeners: Mutex::new(Vec::new()),
            change_listeners: ChangeListeners::new("a change listener of this subscription"),
            feeds: Mutex::new(feeds),
            _item: PhantomData,
        }
    }

    pub(crate) fn as_ptr(&self) -> *mut sys::dxfg_v4_subscription_t {
        self.handle.as_ptr()
    }

    /// The feeds it is attached to.
    pub(crate) fn feeds(&self) -> &Mutex<Vec<DXFeed>> {
        &self.feeds
    }

    /// Attaches the subscription to `feed`, as [`DXFeed::attach_subscription`] does (`attach`). A subscription can be
    /// attached to several feeds; its listeners cannot tell which feed an event came from.
    pub fn attach(&self, feed: &DXFeed) -> Result<()> {
        feed.attach_subscription(self)
    }

    /// Detaches the subscription from `feed`, as [`DXFeed::detach_subscription`] does (`detach`).
    pub fn detach(&self, feed: &DXFeed) -> Result<()> {
        feed.detach_subscription(self)
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
        let list = arena.list(symbols, false)?;
        let s = self.handle.as_ptr();
        // SAFETY: the subscription and the list are live; the SDK only reads the list.
        call(|t| unsafe { sys::dxfg_v4_DXFeedSubscription_addSymbols(t, s, list) })
    }

    /// Removes symbols; takes the same iterables as [`add_symbols`](Self::add_symbols).
    #[doc(alias = "removeSymbols")]
    pub fn remove_symbols(&self, symbols: impl IntoIterator<Item: Into<Symbol>>) -> Result<()> {
        let mut arena = SymbolArena::new();
        let list = arena.list(symbols, false)?;
        let s = self.handle.as_ptr();
        // SAFETY: the subscription and the list are live; the SDK only reads the list.
        call(|t| unsafe { sys::dxfg_v4_DXFeedSubscription_removeSymbols(t, s, list) })
    }

    /// Replaces the symbols with `symbols`, which takes the same iterables as [`add_symbols`](Self::add_symbols)
    /// (`setSymbols`). The change listeners see the symbols added and removed before this returns.
    #[doc(alias = "setSymbols")]
    pub fn set_symbols(&self, symbols: impl IntoIterator<Item: Into<Symbol>>) -> Result<()> {
        let mut arena = SymbolArena::new();
        let list = arena.list(symbols, false)?;
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

    /// Closes the subscription: it detaches from the feeds and lets go of them, and its listeners get no new events,
    /// except possibly one batch already under way. Closing again does nothing.
    pub fn close(&self) -> Result<()> {
        let mut feeds = crate::lock(&self.feeds);
        let s = self.handle.as_ptr();
        // SAFETY: the subscription is live.
        call(|t| unsafe { sys::dxfg_v4_DXFeedSubscription_close(t, s) })?;
        feeds.clear();
        Ok(())
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

    /// The controller of the subscription (`getSubscriptionController`): it attaches the subscription to feeds and
    /// detaches it, and changes how the events are delivered. Experimental in Java.
    #[doc(alias = "getSubscriptionController")]
    pub fn get_subscription_controller(&self) -> Result<SubscriptionController<'_>> {
        let s = self.handle.as_ptr();
        // SAFETY: the subscription is live and the out-pointer is valid for writes.
        let handle =
            Handle::create(|t, out| unsafe { sys::dxfg_v4_DXFeedSubscription_getSubscriptionController(t, s, out) })?;
        Ok(SubscriptionController::new(handle, &self.feeds, Some(&self.handle)))
    }

    /// The executor that calls the listeners; `None` if it is the executor of the feed (`getExecutor`).
    #[doc(alias = "getExecutor")]
    pub fn get_executor(&self) -> Result<Option<Executor>> {
        let s = self.handle.as_ptr();
        // SAFETY: the subscription is live and the out-pointer is valid for writes.
        let handle =
            Handle::create_optional(|t, out| unsafe { sys::dxfg_v4_DXFeedSubscription_getExecutor(t, s, out) })?;
        Ok(handle.map(Executor::from_handle))
    }

    /// Changes the executor that calls the listeners; `None` for the executor of the feed (`setExecutor`).
    #[doc(alias = "setExecutor")]
    pub fn set_executor(&self, executor: Option<&Executor>) -> Result<()> {
        let s = self.handle.as_ptr();
        let e = executor.map_or(ptr::null_mut(), |executor| executor.handle.as_ptr());
        // SAFETY: the subscription and the executor are live.
        call(|t| unsafe { sys::dxfg_v4_DXFeedSubscription_setExecutor(t, s, e) })
    }

    /// The aggregation period that was applied to the events delivered to the listeners: the larger of this period
    /// and the aggregation period of the endpoint (`getAggregationPeriodInfo`). It is
    /// [`TimePeriodInfo::UNKNOWN`] until the first events arrive.
    #[doc(alias = "getAggregationPeriodInfo")]
    pub fn get_aggregation_period_info(&self) -> Result<TimePeriodInfo> {
        let (s, mut min, mut max) = (self.handle.as_ptr(), 0, 0);
        // SAFETY: the subscription is live and the out-pointers are valid for writes.
        call(|t| unsafe { sys::dxfg_v4_DXFeedSubscription_getAggregationPeriodInfo(t, s, &mut min, &mut max) })?;
        if min < 0 || max < 0 {
            return Ok(TimePeriodInfo::UNKNOWN);
        }
        TimePeriodInfo::value_of(min, max)
    }

    /// How long the events are gathered before the listeners are called; `None` if no period was set
    /// (`getAggregationPeriod`).
    #[doc(alias = "getAggregationPeriod")]
    pub fn get_aggregation_period(&self) -> Result<Option<TimePeriod>> {
        let s = self.handle.as_ptr();
        // SAFETY: the subscription is live and the out-pointer is valid for writes.
        let period = Handle::create_optional(|t, out| unsafe {
            sys::dxfg_v4_DXFeedSubscription_getAggregationPeriod(t, s, out)
        })?;
        period.as_ref().map(TimePeriod::from_sdk).transpose()
    }

    /// Changes the aggregation period; `None` clears it (`setAggregationPeriod`). The new period takes effect after
    /// the current one elapses; the effective period is the larger of this period and the aggregation period of the
    /// endpoint.
    ///
    /// Returns [`Error::InvalidArgument`] for a period longer than
    /// [`AGGREGATION_PERIOD_MAX_VALUE`](DXFeedSubscription::AGGREGATION_PERIOD_MAX_VALUE) and [`Error::Java`] for a
    /// negative one.
    #[doc(alias = "setAggregationPeriod")]
    pub fn set_aggregation_period(&self, period: Option<TimePeriod>) -> Result<()> {
        check_aggregation_period(period)?;
        let period = period.map(TimePeriod::to_sdk).transpose()?;
        let s = self.handle.as_ptr();
        let p = period.as_ref().map_or(ptr::null_mut(), Handle::as_ptr);
        // SAFETY: the subscription and the period, if any, are live.
        call(|t| unsafe { sys::dxfg_v4_DXFeedSubscription_setAggregationPeriod(t, s, p) })
    }

    /// The most events of one call of a listener (`getEventsBatchLimit`):
    /// [`OPTIMAL_BATCH_LIMIT`](DXFeedSubscription::OPTIMAL_BATCH_LIMIT), the default, lets the SDK choose and
    /// [`MAX_BATCH_LIMIT`](DXFeedSubscription::MAX_BATCH_LIMIT) means no limit. The limit is not strict.
    #[doc(alias = "getEventsBatchLimit")]
    pub fn get_events_batch_limit(&self) -> Result<i32> {
        let (s, mut limit) = (self.handle.as_ptr(), 0);
        // SAFETY: the subscription is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_DXFeedSubscription_getEventsBatchLimit(t, s, &mut limit) })?;
        Ok(limit)
    }

    /// Changes the most events of one call of a listener (`setEventsBatchLimit`); returns
    /// [`Error::InvalidArgument`] for a negative limit.
    #[doc(alias = "setEventsBatchLimit")]
    pub fn set_events_batch_limit(&self, limit: i32) -> Result<()> {
        check_events_batch_limit(limit)?;
        let s = self.handle.as_ptr();
        // SAFETY: the subscription is live.
        call(|t| unsafe { sys::dxfg_v4_DXFeedSubscription_setEventsBatchLimit(t, s, limit) })
    }

    /// How many events were dropped because of a buffer overflow since the subscription was created
    /// (`getTotalDroppedEvents`). Experimental in Java.
    #[doc(alias = "getTotalDroppedEvents")]
    pub fn get_total_dropped_events(&self) -> Result<i64> {
        let (s, mut dropped) = (self.handle.as_ptr(), 0);
        // SAFETY: the subscription is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_DXFeedSubscription_getTotalDroppedEvents(t, s, &mut dropped) })?;
        Ok(dropped)
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

impl DXFeedSubscription {
    /// The longest aggregation period, 24 hours (`AGGREGATION_PERIOD_MAX_VALUE`).
    pub const AGGREGATION_PERIOD_MAX_VALUE: TimePeriod = TimePeriod::value_of_millis(24 * 3600 * 1000);

    /// The events batch limit that lets the SDK choose (`OPTIMAL_BATCH_LIMIT`).
    pub const OPTIMAL_BATCH_LIMIT: i32 = 0;

    /// The events batch limit that means no limit (`MAX_BATCH_LIMIT`).
    pub const MAX_BATCH_LIMIT: i32 = i32::MAX;
}

/// An SDK list of the classes of event types, which the SDK only reads.
pub(crate) struct ClassList {
    _classes: Vec<sys::dxfg_event_clazz_t>,
    pointers: Vec<*mut sys::dxfg_event_clazz_t>,
    list: sys::dxfg_v4_event_clazz_list,
}

impl ClassList {
    /// The list of `types`; returns [`Error::InvalidArgument`] if `types` is empty.
    pub(crate) fn new(types: &[EventType]) -> Result<Self> {
        if types.is_empty() {
            return Err(Error::InvalidArgument(
                "a subscription needs at least one event type".into(),
            ));
        }
        let mut classes: Vec<sys::dxfg_event_clazz_t> = types.iter().map(|t| t.to_sys()).collect();
        let pointers: Vec<*mut sys::dxfg_event_clazz_t> = classes.iter_mut().map(|c| c as *mut _).collect();
        Ok(ClassList {
            _classes: classes,
            pointers,
            list: sys::dxfg_v4_event_clazz_list {
                size: 0,
                elements: ptr::null_mut(),
            },
        })
    }

    /// The SDK list, valid while `self` lives and is not used again.
    pub(crate) fn as_sys(&mut self) -> *mut sys::dxfg_v4_event_clazz_list {
        self.list = sys::dxfg_v4_event_clazz_list {
            size: self.pointers.len() as i32,
            elements: self.pointers.as_mut_ptr(),
        };
        &mut self.list
    }
}

/// Checks an events batch limit as Java does: it must not be negative.
pub(crate) fn check_events_batch_limit(limit: i32) -> Result<()> {
    if limit < 0 {
        return Err(Error::InvalidArgument(format!(
            "the events batch limit must not be negative, got: {limit}"
        )));
    }
    Ok(())
}

/// Checks an aggregation period as Java does: it must not exceed
/// [`DXFeedSubscription::AGGREGATION_PERIOD_MAX_VALUE`].
pub(crate) fn check_aggregation_period(period: Option<TimePeriod>) -> Result<()> {
    match period {
        Some(period) if period > DXFeedSubscription::AGGREGATION_PERIOD_MAX_VALUE => Err(Error::InvalidArgument(
            format!("Aggregation period is too large: {period}"),
        )),
        _ => Ok(()),
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
/// [`DXFeed::create_time_series_subscription`] or, for several event types and [`Event`],
/// [`DXFeed::create_time_series_subscription_with_types`].
///
/// It is a [`DXFeedSubscription`], whose methods it has through `Deref`, that wraps each symbol it adds in a
/// [`Symbol::TimeSeriesSubscription`] from the time of [`set_from_time`](Self::set_from_time).
pub struct DXFeedTimeSeriesSubscription<E: SubscriptionItem = Event> {
    subscription: DXFeedSubscription<E>,
    /// The time of the last `set_from_time`, read without a lock: change listeners may read it while it is set.
    from_time: AtomicI64,
    /// Serializes `set_from_time`, which keeps `from_time` in step with the SDK.
    setting: Mutex<()>,
}

impl<E: SubscriptionItem> DXFeedTimeSeriesSubscription<E> {
    pub(crate) fn new(subscription: DXFeedSubscription<E>) -> Self {
        DXFeedTimeSeriesSubscription {
            subscription,
            from_time: AtomicI64::new(i64::MAX),
            setting: Mutex::new(()),
        }
    }

    /// The time, in milliseconds since the Unix epoch, of the first events (`getFromTime`): that of the last
    /// [`set_from_time`](Self::set_from_time), or `i64::MAX` before it, when no events come.
    ///
    /// The SDK has no such getter, so this is the time this value was given.
    #[doc(alias = "getFromTime")]
    pub fn get_from_time(&self) -> i64 {
        self.from_time.load(Ordering::Acquire)
    }

    /// Sets the time, in milliseconds since the Unix epoch, of the first events of all its symbols, those added before
    /// and after (`setFromTime`).
    ///
    /// As in Java, the change listeners see the new time; they must not call this method of the same subscription.
    #[doc(alias = "setFromTime")]
    pub fn set_from_time(&self, from_time: i64) -> Result<()> {
        let _setting = crate::lock(&self.setting);
        let previous = self.from_time.swap(from_time, Ordering::AcqRel);
        let s = self
            .subscription
            .handle
            .as_ptr()
            .cast::<sys::dxfg_v4_time_series_subscription_t>();
        // SAFETY: the subscription was created as a time-series subscription and is live.
        let result = call(|t| unsafe { sys::dxfg_v4_DXFeedTimeSeriesSubscription_setFromTime(t, s, from_time) });
        if result.is_err() {
            self.from_time.store(previous, Ordering::Release);
        }
        result
    }
}

impl<E: SubscriptionItem> crate::observable::sealed::Sealed for DXFeedTimeSeriesSubscription<E> {}

impl<E: SubscriptionItem> Observable for DXFeedTimeSeriesSubscription<E> {
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

impl<E: SubscriptionItem> Deref for DXFeedTimeSeriesSubscription<E> {
    type Target = DXFeedSubscription<E>;

    fn deref(&self) -> &DXFeedSubscription<E> {
        &self.subscription
    }
}
