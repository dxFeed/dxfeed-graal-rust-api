//! The feed of an endpoint.

use std::sync::{Arc, Mutex, OnceLock};

use dxfeed_graal_sys as sys;

use crate::endpoint::{DXEndpoint, EndpointState};
use crate::error::{Error, JavaException, Result};
use crate::event::sdk_event::{SdkSymbol, needs_candle_symbol, take_event};
use crate::event::{
    Event, EventKind, EventType, IndexedEvent, IndexedEventSource, LastingEvent, SysEvents, TimeSeriesEvent,
};
use crate::handle::Handle;
use crate::promise::Promise;
use crate::subscription::{DXFeedSubscription, DXFeedTimeSeriesSubscription};
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
        crate::get_or_try_init(&INSTANCE, &INIT, || Ok(DXEndpoint::get_instance()?.feed()))
    }

    /// Creates a subscription to events of type `E`, such as [`Quote`](crate::event::Quote), attached to this feed;
    /// its listeners receive the views of the type, such as `&QuoteRef`.
    ///
    /// ```no_run
    /// # fn main() -> dxfeed_graal::Result<()> {
    /// use dxfeed_graal::DXEndpoint;
    /// use dxfeed_graal::event::Quote;
    ///
    /// let subscription = DXEndpoint::create()?.feed().create_subscription::<Quote>()?;
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
    /// let subscription = DXEndpoint::create()?.feed().create_subscription::<Event>()?;
    /// # Ok(())
    /// # }
    /// ```
    #[doc(alias = "createSubscription")]
    pub fn create_subscription<E: EventKind>(&self) -> Result<DXFeedSubscription<E>> {
        let feed = self.state.feed.as_ptr();
        // SAFETY: the feed is live and the out-pointer is valid for writes.
        let handle =
            Handle::create(|t, out| unsafe { sys::dxfg_v4_DXFeed_createSubscription(t, feed, E::TYPE.to_sys(), out) })?;
        Ok(DXFeedSubscription::new(handle, self.clone()))
    }

    /// Creates a subscription to events of `types`, attached to this feed; its listeners receive
    /// [`EventRef`](crate::EventRef).
    ///
    /// Returns [`Error::InvalidArgument`] if `types` is empty.
    #[doc(alias = "createSubscription")]
    pub fn create_subscription_with_types(&self, types: &[EventType]) -> Result<DXFeedSubscription<Event>> {
        if types.is_empty() {
            return Err(Error::InvalidArgument(
                "a subscription needs at least one event type".into(),
            ));
        }
        let mut classes: Vec<sys::dxfg_event_clazz_t> = types.iter().map(|t| t.to_sys()).collect();
        let mut pointers: Vec<*mut sys::dxfg_event_clazz_t> = classes.iter_mut().map(|c| c as *mut _).collect();
        let mut list = sys::dxfg_v4_event_clazz_list {
            size: pointers.len() as i32,
            elements: pointers.as_mut_ptr(),
        };
        let feed = self.state.feed.as_ptr();
        // SAFETY: the feed and the list are live; the SDK only reads the list. The out-pointer is valid for writes.
        let handle = Handle::create(|t, out| unsafe {
            sys::dxfg_v4_DXFeed_createSubscriptionWithEventTypes(t, feed, &mut list, out)
        })?;
        Ok(DXFeedSubscription::new(handle, self.clone()))
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
    /// let subscription = DXEndpoint::create()?.feed().create_time_series_subscription::<Candle>()?;
    /// subscription.set_from_time(0)?;
    /// subscription.add_symbols(["AAPL{=d}"])?;
    /// # Ok(())
    /// # }
    /// ```
    #[doc(alias = "createTimeSeriesSubscription")]
    pub fn create_time_series_subscription<E: TimeSeriesEvent>(&self) -> Result<DXFeedTimeSeriesSubscription<E>> {
        let feed = self.state.feed.as_ptr();
        // SAFETY: the feed is live and the out-pointer is valid for writes; a time-series subscription starts with
        // the fields of a subscription.
        let handle = Handle::create(|t, out: *mut *mut sys::dxfg_v4_subscription_t| unsafe {
            sys::dxfg_v4_DXFeed_createTimeSeriesSubscription(t, feed, E::TYPE.to_sys(), out.cast())
        })?;
        Ok(DXFeedTimeSeriesSubscription::new(DXFeedSubscription::new(
            handle,
            self.clone(),
        )))
    }

    /// Requests the time-series events of type `E` for `symbol` from `from_time` to `to_time`, in milliseconds since
    /// the Unix epoch, and returns the promise of the list, ordered by time (`getTimeSeriesPromise`).
    ///
    /// The request needs no subscription. A string symbol of candles is passed as a
    /// [`CandleSymbol`](crate::event::CandleSymbol). The promise completes exceptionally when the events are permanently
    /// unavailable; it keeps the endpoint open while it lives.
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
    ///     .feed()
    ///     .get_time_series_promise::<Candle>("AAPL{=d}", to - 20 * 86_400_000, to)?
    ///     .wait(Duration::from_secs(5))?;
    /// # Ok(())
    /// # }
    /// ```
    #[doc(alias = "getTimeSeriesPromise")]
    pub fn get_time_series_promise<E: TimeSeriesEvent>(
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
    pub fn get_indexed_events_promise<E: IndexedEvent>(
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
    /// let quote = DXEndpoint::create()?.feed().get_last_event_if_subscribed::<Quote>("AAPL")?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// ```compile_fail
    /// # fn main() -> dxfeed_graal::Result<()> {
    /// use dxfeed_graal::DXEndpoint;
    /// use dxfeed_graal::event::TimeAndSale;
    ///
    /// let sale = DXEndpoint::create()?.feed().get_last_event_if_subscribed::<TimeAndSale>("AAPL")?;
    /// # Ok(())
    /// # }
    /// ```
    #[doc(alias = "getLastEventIfSubscribed")]
    pub fn get_last_event_if_subscribed<E: LastingEvent>(&self, symbol: &str) -> Result<Option<E>> {
        let mut sdk_symbol = SdkSymbol::new::<E>(symbol)?;
        let symbol = sdk_symbol.as_mut_ptr();
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
    /// let quote = endpoint.feed().get_last_event_promise::<Quote>("AAPL")?.wait(Duration::from_secs(5))?;
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
    /// let promise = DXEndpoint::create()?.feed().get_last_event_promise::<Order>("AAPL")?;
    /// # Ok(())
    /// # }
    /// ```
    #[doc(alias = "getLastEventPromise")]
    pub fn get_last_event_promise<E: LastingEvent>(&self, symbol: &str) -> Result<Promise<E>> {
        let mut sdk_symbol = SdkSymbol::new::<E>(symbol)?;
        let symbol = sdk_symbol.as_mut_ptr();
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
    /// let quote = DXEndpoint::create()?.feed().get_last_event(&Quote::new("AAPL"))?;
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
    /// let order = DXEndpoint::create()?.feed().get_last_event(&Order::new("AAPL"))?;
    /// # Ok(())
    /// # }
    /// ```
    #[doc(alias = "getLastEvent")]
    pub fn get_last_event<E: LastingEvent>(&self, event: &E) -> Result<E> {
        let sys_events = SysEvents::new(&[event.clone().into()])?;
        let event = sys_events.pointers()[0];
        let feed = self.state.feed.as_ptr();
        // SAFETY: the feed and the event are live; the SDK only reads the event. The out-pointer is valid for writes.
        take_event(|t, out| unsafe { sys::dxfg_v4_DXFeed_getLastEvent(t, feed, event, out) })?
            .ok_or_else(|| Error::Java(JavaException::null_result()))
    }
}
