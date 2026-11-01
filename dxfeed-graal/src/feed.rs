//! The feed of an endpoint.

use std::ptr;
use std::sync::{Arc, Mutex, OnceLock};

use dxfeed_graal_sys as sys;

use crate::endpoint::{DXEndpoint, EndpointState};
use crate::error::{Error, JavaException, Result};
use crate::event::SubscriptionItem;
use crate::event::sdk_event::{needs_candle_symbol, take_event, take_events};
use crate::event::{
    Event, EventKind, EventType, IndexedEvent, IndexedEventSource, LastingEvent, SysEvents, TimeSeriesEvent,
};
use crate::handle::Handle;
use crate::isolate::{self, call};
use crate::promise::Promise;
use crate::subscription::{ClassList, DXFeedSubscription, DXFeedTimeSeriesSubscription};
use crate::subscription_controller::{attach_feed, detach_feed};
use crate::symbol::{Symbol, SymbolArena};

/// The feed of a [`DXEndpoint`](crate::DXEndpoint): creates subscriptions to events and returns the last events of the
/// [`LastingEvent`] types (`DXFeed`).
///
/// It keeps its endpoint open, as do its subscriptions; cloning it is cheap.
#[derive(Clone)]
pub struct DXFeed {
    state: Arc<EndpointState>,
}

impl DXFeed {
    /// The SDK feed, valid while `self` lives.
    pub(crate) fn as_ptr(&self) -> *mut sys::dxfg_v4_feed_t {
        self.state.feed.as_ptr()
    }

    /// Whether both are the feed of the same endpoint.
    pub(crate) fn same_as(&self, other: &DXFeed) -> bool {
        Arc::ptr_eq(&self.state, &other.state)
    }

    pub(crate) fn new(state: Arc<EndpointState>) -> Self {
        DXFeed { state }
    }

    /// Returns the feed of the default endpoint of the process, [`DXEndpoint::get_instance`] (`DXFeed.getInstance()`).
    ///
    /// The endpoint is configured on first use by the `dxfeed.properties` file and the system properties, such as
    /// `dxfeed.address`; closing it closes it for the whole process.
    #[doc(alias = "getInstance")]
    pub fn get_instance() -> Result<&'static DXFeed> {
        static INSTANCE: OnceLock<DXFeed> = OnceLock::new();
        static INIT: Mutex<()> = Mutex::new(());
        crate::get_or_try_init(&INSTANCE, &INIT, || Ok(DXEndpoint::get_instance()?.get_feed()))
    }

    /// Creates a subscription to events of type `E`, such as [`Quote`](crate::event::Quote), attached to this feed;
    /// its listeners receive the views of the type, such as `&QuoteRef`.
    ///
    /// ```no_run
    /// # fn main() -> dxfeed_graal::Result<()> {
    /// use dxfeed_graal::DXEndpoint;
    /// use dxfeed_graal::event::Quote;
    ///
    /// let subscription = DXEndpoint::create()?.get_feed().create_subscription::<Quote>()?;
    /// subscription.add_event_listener(|quotes| quotes.for_each(|quote| println!("{}", quote.symbol())))?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// [`Event`](crate::Event) is not an event type of its own, so a subscription to it does not compile; a
    /// subscription to several event types is created with
    /// [`create_subscription_with_types`](Self::create_subscription_with_types) instead:
    ///
    /// ```compile_fail
    /// # fn main() -> dxfeed_graal::Result<()> {
    /// use dxfeed_graal::{DXEndpoint, Event};
    ///
    /// let subscription = DXEndpoint::create()?.get_feed().create_subscription::<Event>()?;
    /// # Ok(())
    /// # }
    /// ```
    #[doc(alias = "createSubscription")]
    pub fn create_subscription<E: EventKind>(&self) -> Result<DXFeedSubscription<E>> {
        let feed = self.state.feed.as_ptr();
        // SAFETY: the feed is live and the out-pointer is valid for writes.
        let handle =
            Handle::create(|t, out| unsafe { sys::dxfg_v4_DXFeed_createSubscription(t, feed, E::TYPE.to_sys(), out) })?;
        Ok(DXFeedSubscription::from_sdk(handle, vec![self.clone()]))
    }

    /// Creates a subscription to events of `types`, attached to this feed; its listeners receive
    /// [`EventRef`](crate::EventRef).
    ///
    /// Returns [`Error::InvalidArgument`] if `types` is empty.
    #[doc(alias = "createSubscription")]
    pub fn create_subscription_with_types(&self, types: &[EventType]) -> Result<DXFeedSubscription<Event>> {
        let mut classes = ClassList::new(types)?;
        let list = classes.as_sys();
        let feed = self.state.feed.as_ptr();
        // SAFETY: the feed and the list are live; the SDK only reads the list. The out-pointer is valid for writes.
        let handle = Handle::create(|t, out| unsafe {
            sys::dxfg_v4_DXFeed_createSubscriptionWithEventTypes(t, feed, list, out)
        })?;
        Ok(DXFeedSubscription::from_sdk(handle, vec![self.clone()]))
    }

    /// Creates a subscription to the time-series events of type `E`, such as [`Candle`](crate::event::Candle), from
    /// a time on, attached to this feed (`createTimeSeriesSubscription`).
    ///
    /// Its symbols get the time of [`set_from_time`](DXFeedTimeSeriesSubscription::set_from_time), and no events
    /// come until it is set (the time is `i64::MAX`); it is used as a [`DXFeedSubscription`] otherwise.
    ///
    /// ```no_run
    /// # fn main() -> dxfeed_graal::Result<()> {
    /// use dxfeed_graal::DXEndpoint;
    /// use dxfeed_graal::event::Candle;
    ///
    /// let subscription = DXEndpoint::create()?.get_feed().create_time_series_subscription::<Candle>()?;
    /// subscription.set_from_time(0)?;
    /// subscription.add_symbols(["AAPL{=d}"])?;
    /// # Ok(())
    /// # }
    /// ```
    #[doc(alias = "createTimeSeriesSubscription")]
    pub fn create_time_series_subscription<E: TimeSeriesEvent + EventKind>(
        &self,
    ) -> Result<DXFeedTimeSeriesSubscription<E>> {
        let feed = self.state.feed.as_ptr();
        // SAFETY: the feed is live and the out-pointer is valid for writes; a time-series subscription starts with
        // the fields of a subscription.
        let handle = Handle::create(|t, out: *mut *mut sys::dxfg_v4_subscription_t| unsafe {
            sys::dxfg_v4_DXFeed_createTimeSeriesSubscription(t, feed, E::TYPE.to_sys(), out.cast())
        })?;
        Ok(DXFeedTimeSeriesSubscription::new(DXFeedSubscription::from_sdk(
            handle,
            vec![self.clone()],
        )))
    }

    /// Creates a subscription to the time-series events of `types` from a time on, attached to this feed; its
    /// listeners receive [`EventRef`](crate::EventRef) (`createTimeSeriesSubscription`).
    ///
    /// Returns [`Error::InvalidArgument`] if `types` is empty or holds a type that is not a
    /// [`TimeSeriesEvent`](crate::event::TimeSeriesEvent).
    #[doc(alias = "createTimeSeriesSubscription")]
    pub fn create_time_series_subscription_with_types(
        &self,
        types: &[EventType],
    ) -> Result<DXFeedTimeSeriesSubscription<Event>> {
        if let Some(event_type) = types.iter().find(|event_type| !event_type.is_time_series()) {
            return Err(Error::InvalidArgument(format!(
                "{event_type} is not a time-series event type"
            )));
        }
        let mut classes = ClassList::new(types)?;
        let list = classes.as_sys();
        let feed = self.state.feed.as_ptr();
        // SAFETY: the feed and the list are live; the SDK only reads the list. The out-pointer is valid for writes; a
        // time-series subscription starts with the fields of a subscription.
        let handle = Handle::create(|t, out: *mut *mut sys::dxfg_v4_subscription_t| unsafe {
            sys::dxfg_v4_DXFeed_createTimeSeriesSubscriptionWithEventTypes(t, feed, list, out.cast())
        })?;
        Ok(DXFeedTimeSeriesSubscription::new(DXFeedSubscription::from_sdk(
            handle,
            vec![self.clone()],
        )))
    }

    /// Attaches `subscription` to this feed, which then delivers the events of its symbols to its listeners; attaching
    /// it again, or a closed subscription, does nothing (`attachSubscription`). The subscription keeps the endpoint of
    /// the feed open until it is detached or closed.
    #[doc(alias = "attachSubscription")]
    pub fn attach_subscription<E: SubscriptionItem>(&self, subscription: &DXFeedSubscription<E>) -> Result<()> {
        let s = subscription.as_ptr();
        attach_feed(subscription.feeds(), self, |f| {
            // SAFETY: the feed and the subscription are live.
            call(|t| unsafe { sys::dxfg_v4_DXFeed_attachSubscription(t, f, s) })?;
            Ok(!subscription.is_closed()?)
        })
    }

    /// Detaches `subscription` from this feed (`detachSubscription`); its symbols stay. Does nothing if it is not
    /// attached to this feed.
    #[doc(alias = "detachSubscription")]
    pub fn detach_subscription<E: SubscriptionItem>(&self, subscription: &DXFeedSubscription<E>) -> Result<()> {
        let s = subscription.as_ptr();
        detach_feed(subscription.feeds(), self, |f| {
            // SAFETY: the feed and the subscription are live.
            call(|t| unsafe { sys::dxfg_v4_DXFeed_detachSubscription(t, f, s) })
        })
    }

    /// Detaches `subscription` from this feed and clears the data the feed keeps for it
    /// (`detachSubscriptionAndClear`); its symbols stay. Java publishes empty events for that, which the SDK may not
    /// deliver to the listeners. Does nothing if it is not attached to this feed.
    #[doc(alias = "detachSubscriptionAndClear")]
    pub fn detach_subscription_and_clear<E: SubscriptionItem>(
        &self,
        subscription: &DXFeedSubscription<E>,
    ) -> Result<()> {
        let s = subscription.as_ptr();
        detach_feed(subscription.feeds(), self, |f| {
            // SAFETY: the feed and the subscription are live.
            call(|t| unsafe { sys::dxfg_v4_DXFeed_detachSubscriptionAndClear(t, f, s) })
        })
    }

    /// Requests the time-series events of type `E` for `symbol` from `from_time` to `to_time`, in milliseconds since
    /// the Unix epoch, and returns the promise of the list, ordered by time (`getTimeSeriesPromise`).
    ///
    /// The request needs no subscription. A string symbol of candles is passed as a
    /// [`CandleSymbol`](crate::event::CandleSymbol). The promise completes exceptionally when the events are
    /// permanently unavailable; it keeps the endpoint open while it lives.
    ///
    /// ```no_run
    /// # fn main() -> dxfeed_graal::Result<()> {
    /// use std::time::Duration;
    ///
    /// use dxfeed_graal::DXEndpoint;
    /// use dxfeed_graal::event::Candle;
    ///
    /// let endpoint = DXEndpoint::create()?;
    /// endpoint.connect("demo.dxfeed.com:7300")?;
    /// let to = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
    /// let candles = endpoint
    ///     .get_feed()
    ///     .get_time_series_promise::<Candle>("AAPL{=d}", to - 20 * 86_400_000, to)?
    ///     .wait(Duration::from_secs(5))?;
    /// # Ok(())
    /// # }
    /// ```
    #[doc(alias = "getTimeSeriesPromise")]
    pub fn get_time_series_promise<E: TimeSeriesEvent + EventKind>(
        &self,
        symbol: impl Into<Symbol>,
        from_time: i64,
        to_time: i64,
    ) -> Result<Promise<Vec<E>>> {
        let mut arena = SymbolArena::new();
        let symbol = arena.symbol(&symbol.into(), needs_candle_symbol(E::TYPE))?;
        let feed = self.state.feed.as_ptr();
        // SAFETY: the feed and the symbol are live; the SDK only reads the symbol. The out-pointer is valid for writes,
        // and a promise of a list starts with the fields of a promise.
        let handle = Handle::create(|t, out: *mut *mut sys::dxfg_v4_promise_t| unsafe {
            sys::dxfg_v4_DXFeed_getTimeSeriesPromise(t, feed, E::TYPE.to_sys(), symbol, from_time, to_time, out.cast())
        })?;
        Ok(Promise::new(handle, self.clone()))
    }

    /// Requests the indexed events of type `E` for `symbol` from `source` and returns the promise of the list, ordered
    /// by index (`getIndexedEventsPromise`).
    ///
    /// The request needs no subscription. The promise completes with the snapshot of the events that the uplink sends;
    /// it keeps the endpoint open while it lives.
    #[doc(alias = "getIndexedEventsPromise")]
    pub fn get_indexed_events_promise<E: IndexedEvent + EventKind>(
        &self,
        symbol: impl Into<Symbol>,
        source: IndexedEventSource,
    ) -> Result<Promise<Vec<E>>> {
        let mut arena = SymbolArena::new();
        let symbol = arena.symbol(&symbol.into(), needs_candle_symbol(E::TYPE))?;
        let source = arena.source(source);
        let feed = self.state.feed.as_ptr();
        // SAFETY: the feed, the symbol and the source are live; the SDK only reads them. The out-pointer is valid for
        // writes, and a promise of a list starts with the fields of a promise.
        let handle = Handle::create(|t, out: *mut *mut sys::dxfg_v4_promise_t| unsafe {
            sys::dxfg_v4_DXFeed_getIndexedEventsPromise(t, feed, E::TYPE.to_sys(), symbol, source, out.cast())
        })?;
        Ok(Promise::new(handle, self.clone()))
    }

    /// Returns the last event of type `E` for `symbol` from the local cache of the feed, or `None` if no subscription
    /// of this feed is subscribed to them (`DXFeed.getLastEventIfSubscribed`).
    ///
    /// The call is not remote; a wildcard subscription does not count. An event that has not arrived yet is returned
    /// with only its symbol set, as `E::new(symbol)` with the SDK defaults: `NaN` prices and sizes, other fields zero
    /// or `None`. A [`STREAM_FEED`](crate::Role::StreamFeed) endpoint always returns `None`. A candle symbol such as
    /// `AAPL{=d}` is parsed by the SDK; a malformed one fails with [`Error::Java`].
    ///
    /// Only the [`LastingEvent`] types have a last event:
    ///
    /// ```no_run
    /// # fn main() -> dxfeed_graal::Result<()> {
    /// use dxfeed_graal::DXEndpoint;
    /// use dxfeed_graal::event::Quote;
    ///
    /// let quote = DXEndpoint::create()?.get_feed().get_last_event_if_subscribed::<Quote>("AAPL")?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// ```compile_fail
    /// # fn main() -> dxfeed_graal::Result<()> {
    /// use dxfeed_graal::DXEndpoint;
    /// use dxfeed_graal::event::TimeAndSale;
    ///
    /// let sale = DXEndpoint::create()?.get_feed().get_last_event_if_subscribed::<TimeAndSale>("AAPL")?;
    /// # Ok(())
    /// # }
    /// ```
    #[doc(alias = "getLastEventIfSubscribed")]
    pub fn get_last_event_if_subscribed<E: LastingEvent + EventKind>(
        &self,
        symbol: impl Into<Symbol>,
    ) -> Result<Option<E>> {
        let mut arena = SymbolArena::new();
        let symbol = arena.symbol(&symbol.into(), needs_candle_symbol(E::TYPE))?;
        let feed = self.state.feed.as_ptr();
        // SAFETY: the feed and the symbol are live; the SDK only reads the symbol. The out-pointer is valid for writes.
        take_event(|t, out| unsafe {
            sys::dxfg_v4_DXFeed_getLastEventIfSubscribed(t, feed, E::TYPE.to_sys(), symbol, out)
        })
    }

    /// Requests the last event of type `E` for `symbol` from the uplink and returns the promise of the result
    /// (`DXFeed.getLastEventPromise`).
    ///
    /// The request needs no subscription. It completes when the event is known, which may take arbitrarily long, so
    /// [`Promise::wait`] takes a timeout. It completes exceptionally when the event is permanently unavailable and on a
    /// [`STREAM_FEED`](crate::Role::StreamFeed) endpoint. Closing the endpoint does not complete it, so only the
    /// timeout ends a wait then. The promise keeps the endpoint open while it lives. A candle symbol such as
    /// `AAPL{=d}` is parsed by the SDK; a malformed one fails with [`Error::Java`].
    ///
    /// ```no_run
    /// # fn main() -> dxfeed_graal::Result<()> {
    /// use std::time::Duration;
    ///
    /// use dxfeed_graal::DXEndpoint;
    /// use dxfeed_graal::event::Quote;
    ///
    /// let endpoint = DXEndpoint::create()?;
    /// endpoint.connect("demo.dxfeed.com:7300")?;
    /// let quote = endpoint.get_feed().get_last_event_promise::<Quote>("AAPL")?.wait(Duration::from_secs(5))?;
    /// println!("{quote}");
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// Only the [`LastingEvent`] types have a last event:
    ///
    /// ```compile_fail
    /// # fn main() -> dxfeed_graal::Result<()> {
    /// use dxfeed_graal::DXEndpoint;
    /// use dxfeed_graal::event::Order;
    ///
    /// let promise = DXEndpoint::create()?.get_feed().get_last_event_promise::<Order>("AAPL")?;
    /// # Ok(())
    /// # }
    /// ```
    #[doc(alias = "getLastEventPromise")]
    pub fn get_last_event_promise<E: LastingEvent + EventKind>(&self, symbol: impl Into<Symbol>) -> Result<Promise<E>> {
        let mut arena = SymbolArena::new();
        let symbol = arena.symbol(&symbol.into(), needs_candle_symbol(E::TYPE))?;
        let feed = self.state.feed.as_ptr();
        // SAFETY: the feed and the symbol are live; the SDK only reads the symbol. The out-pointer is valid for writes.
        let handle = Handle::create(|t, out: *mut *mut sys::dxfg_v4_promise_t| unsafe {
            sys::dxfg_v4_DXFeed_getLastEventPromise(t, feed, E::TYPE.to_sys(), symbol, out.cast())
        })?;
        Ok(Promise::new(handle, self.clone()))
    }

    /// Returns a new event with the last values of the type and symbol of `event` from the local cache of the feed, or
    /// a copy of `event` when they are not known (`DXFeed.getLastEvent`).
    ///
    /// The values are known when a subscription of this feed is subscribed to the type and symbol and the event has
    /// arrived; a returned copy cannot be told apart from an event that has the same values. Use
    /// [`get_last_event_if_subscribed`](Self::get_last_event_if_subscribed) to tell whether there is a subscription.
    ///
    /// Fails with [`Error::InvalidArgument`] when a string of `event` holds a NUL byte or an exchange code does not fit
    /// a Java `char`.
    ///
    /// ```no_run
    /// # fn main() -> dxfeed_graal::Result<()> {
    /// use dxfeed_graal::DXEndpoint;
    /// use dxfeed_graal::event::Quote;
    ///
    /// let quote = DXEndpoint::create()?.get_feed().get_last_event(&Quote::new("AAPL"))?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// Only the [`LastingEvent`] types have a last event:
    ///
    /// ```compile_fail
    /// # fn main() -> dxfeed_graal::Result<()> {
    /// use dxfeed_graal::DXEndpoint;
    /// use dxfeed_graal::event::Order;
    ///
    /// let order = DXEndpoint::create()?.get_feed().get_last_event(&Order::new("AAPL"))?;
    /// # Ok(())
    /// # }
    /// ```
    #[doc(alias = "getLastEvent")]
    pub fn get_last_event<E: LastingEvent + EventKind>(&self, event: &E) -> Result<E> {
        let sys_events = SysEvents::new(&[event.clone().into()])?;
        let event = sys_events.pointers()[0];
        let feed = self.state.feed.as_ptr();
        // SAFETY: the feed and the event are live; the SDK only reads the event. The out-pointer is valid for writes.
        take_event(|t, out| unsafe { sys::dxfg_v4_DXFeed_getLastEvent(t, feed, event, out) })?
            .ok_or_else(|| Error::Java(JavaException::null_result()))
    }

    /// Returns new events with the last values of the types and symbols of `events`, in their order, as
    /// [`get_last_event`](Self::get_last_event) does for one (`DXFeed.getLastEvents`).
    ///
    /// Java fills the given events; this method leaves them and returns the filled copies.
    ///
    /// Fails with [`Error::InvalidArgument`] when a string of an event holds a NUL byte or an exchange code does not
    /// fit a Java `char`.
    #[doc(alias = "getLastEvents")]
    pub fn get_last_events<E: LastingEvent + EventKind>(&self, events: &[E]) -> Result<Vec<E>> {
        if events.is_empty() {
            return Ok(Vec::new());
        }
        let events: Vec<Event> = events.iter().cloned().map(Into::into).collect();
        let mut sys_events = SysEvents::new(&events)?;
        let list = sys_events.list();
        let feed = self.state.feed.as_ptr();
        // SAFETY: the feed, the list and its events are live; the SDK only reads them. The out-pointer is valid for
        // writes.
        let last = take_events(|t, out| unsafe { sys::dxfg_v4_DXFeed_getLastEvents(t, feed, &list, out) })?;
        last.ok_or_else(|| Error::Java(JavaException::null_result()))
    }

    /// Requests the last events of type `E` for `symbols` from the uplink and returns their promises, in the order of
    /// `symbols` (`DXFeed.getLastEventsPromises`).
    ///
    /// It is much faster than one [`get_last_event_promise`](Self::get_last_event_promise) per symbol; wait for all
    /// of them with [`Promises::all_of`](crate::Promises::all_of). Each promise keeps the endpoint open while it lives.
    #[doc(alias = "getLastEventsPromises")]
    pub fn get_last_events_promises<E: LastingEvent + EventKind>(
        &self,
        symbols: impl IntoIterator<Item: Into<Symbol>>,
    ) -> Result<Vec<Promise<E>>> {
        let mut arena = SymbolArena::new();
        let symbols = arena.list(symbols, needs_candle_symbol(E::TYPE))?;
        let feed = self.state.feed.as_ptr();
        let mut list: *mut sys::dxfg_v4_promise_list = ptr::null_mut();
        // SAFETY: the feed and the symbols are live; the SDK only reads the symbols. The out-pointer is valid for
        // writes.
        call(|t| unsafe { sys::dxfg_v4_DXFeed_getLastEventsPromises(t, feed, E::TYPE.to_sys(), symbols, &mut list) })?;
        let list = PromiseList(list);
        list.handles()?
            .into_iter()
            .map(|handle| Ok(Promise::new(handle, self.clone())))
            .collect()
    }

    /// Returns the indexed events of type `E` for `symbol` from `source` in the local cache of the feed, ordered by
    /// index (`DXFeed.getIndexedEventsIfSubscribed`).
    ///
    /// The call is not remote. The cache has the events only while a subscription of this feed is subscribed to them,
    /// and the list is not a consistent snapshot: use
    /// [`get_indexed_events_promise`](Self::get_indexed_events_promise) for one. The list is empty without such a
    /// subscription, where Java returns `null`, and before the events arrive.
    ///
    /// Fails with [`Error::InvalidArgument`] when `source` is neither [`IndexedEventSource::DEFAULT`] nor an order
    /// source of Java, such as one made by [`IndexedEventSource::new`] with an id that Java does not know.
    #[doc(alias = "getIndexedEventsIfSubscribed")]
    pub fn get_indexed_events_if_subscribed<E: IndexedEvent + EventKind>(
        &self,
        symbol: impl Into<Symbol>,
        source: IndexedEventSource,
    ) -> Result<Vec<E>> {
        let mut arena = SymbolArena::new();
        let symbol = arena.symbol(&symbol.into(), needs_candle_symbol(E::TYPE))?;
        // The SDK finds the source by the name of its `OrderSource`, as `OrderSource.valueOf` does.
        let name = source
            .order_source_name()
            .ok_or_else(|| Error::InvalidArgument(format!("{source} is not a source of indexed events of Java")))?;
        let source = arena.string(&name, "the source")?;
        let feed = self.state.feed.as_ptr();
        // SAFETY: the feed, the symbol and the source are live; the SDK only reads them. The out-pointer is valid for
        // writes.
        let events = take_events(|t, out| unsafe {
            sys::dxfg_v4_DXFeed_getIndexedEventsIfSubscribed(t, feed, E::TYPE.to_sys(), symbol, source, out)
        })?;
        Ok(events.unwrap_or_default())
    }

    /// Returns the time-series events of type `E` for `symbol` from `from_time` to `to_time`, both inclusive, in the
    /// local cache of the feed, ordered by time (`DXFeed.getTimeSeriesIfSubscribed`).
    ///
    /// The call is not remote. The cache has the events only while a subscription of this feed is subscribed to them
    /// from a time on, and the list is not a consistent snapshot: use
    /// [`get_time_series_promise`](Self::get_time_series_promise) for one. The list is empty without such a
    /// subscription, where Java returns `null`, and before the events arrive. A
    /// [`Symbol::TimeSeriesSubscription`] symbol fails with [`Error::Java`].
    #[doc(alias = "getTimeSeriesIfSubscribed")]
    pub fn get_time_series_if_subscribed<E: TimeSeriesEvent + EventKind>(
        &self,
        symbol: impl Into<Symbol>,
        from_time: i64,
        to_time: i64,
    ) -> Result<Vec<E>> {
        let mut arena = SymbolArena::new();
        let symbol = arena.symbol(&symbol.into(), needs_candle_symbol(E::TYPE))?;
        let feed = self.state.feed.as_ptr();
        // SAFETY: the feed and the symbol are live; the SDK only reads the symbol. The out-pointer is valid for writes.
        let events = take_events(|t, out| unsafe {
            sys::dxfg_v4_DXFeed_getTimeSeriesIfSubscribed(t, feed, E::TYPE.to_sys(), symbol, from_time, to_time, out)
        })?;
        Ok(events.unwrap_or_default())
    }
}

/// A list of promises the SDK returned, released with its handles on `Drop`.
struct PromiseList(*mut sys::dxfg_v4_promise_list);

impl PromiseList {
    /// New handles of the promises of the list.
    fn handles(&self) -> Result<Vec<Handle<sys::dxfg_v4_promise_t>>> {
        // SAFETY: the list is NULL or live while `self` lives.
        let Some(list) = (unsafe { self.0.as_ref() }) else {
            return Ok(Vec::new());
        };
        let len = usize::try_from(list.list.size).unwrap_or(0);
        if len == 0 || list.list.elements.is_null() {
            return Ok(Vec::new());
        }
        // SAFETY: the array holds `size` live handles.
        let elements = unsafe { std::slice::from_raw_parts(list.list.elements, len) };
        elements
            .iter()
            .map(|&element| {
                if element.is_null() {
                    return Err(Error::Java(JavaException::null_result()));
                }
                // SAFETY: the element is a live handle of a promise, which stays with the list.
                unsafe { Handle::clone_raw(element.cast::<sys::dxfg_v4_promise_t>()) }
            })
            .collect()
    }
}

impl Drop for PromiseList {
    fn drop(&mut self) {
        let list = self.0;
        if !list.is_null() && !isolate::exiting() {
            // SAFETY: the list came from the SDK and is released once.
            crate::error_manager::report_release(
                "Promise",
                call(|t| unsafe { sys::dxfg_v4_promise_list_release(t, list) }),
            );
        }
    }
}
