//! Market events: borrowed views passed to listeners and owned values for publishing.
//!
//! Every event type has an owned struct, such as [`Quote`], with pub fields and `new(symbol)`, and a view, such as
//! [`QuoteRef`], that a listener receives: it reads the SDK's memory in place and is valid during the call only. The
//! views have an accessor per field; both print the text of Java's `toString`.

#[macro_use]
mod table;
mod flags;
#[macro_use]
mod traits;
mod analytic_order;
mod candle;
mod candle_symbol;
mod configuration;
mod daily_candle;
mod format;
pub(crate) use format::{JavaDouble, year_month_day};
mod greeks;
mod market_maker;
mod message;
mod nuam_order;
mod nuam_time_and_sale;
mod nuam_trade;
mod option_sale;
mod order;
mod order_base;
mod order_imbalance;
mod otc_markets_order;
mod profile;
mod quote;
pub(crate) mod sdk_event;
mod series;
mod source;
mod spread_order;
mod summary;
mod text_configuration;
mod text_message;
mod theo_price;
mod time_and_sale;
mod trade;
mod trade_eth;
mod underlying;

use std::borrow::Cow;
use std::ffi::{CStr, CString, c_char};
use std::fmt::{self, Display, Formatter};
use std::marker::PhantomData;
use std::ptr;

use dxfeed_graal_sys as sys;

pub use analytic_order::{AnalyticOrder, AnalyticOrderRef};
pub use candle::{Candle, CandleRef};
pub use candle_symbol::{
    CandleAlignment, CandleAttribute, CandlePeriod, CandlePrice, CandleSession, CandleSymbol, CandleType,
};
pub use configuration::{Configuration, ConfigurationRef};
pub use daily_candle::{DailyCandle, DailyCandleRef};
pub use flags::{
    AuctionType, Direction, EventFlags, IcebergType, ImbalanceSide, NuamOrderType, NuamTimeInForceType, OrderAction,
    OtcMarketsPriceType, PriceType, Scope, ShortSaleRestriction, Side, TimeAndSaleType, TradingStatus,
};
pub use greeks::{Greeks, GreeksRef};
pub use market_maker::{MarketMaker, MarketMakerRef};
pub use message::{Message, MessageRef};
pub use nuam_order::{NuamOrder, NuamOrderRef};
pub use nuam_time_and_sale::{NuamTimeAndSale, NuamTimeAndSaleRef};
pub use nuam_trade::{NuamTrade, NuamTradeRef};
pub use option_sale::{OptionSale, OptionSaleRef};
pub use order::{Order, OrderRef};
pub use order_base::{OrderBase, OrderBaseRef};
pub use order_imbalance::{OrderImbalance, OrderImbalanceRef};
pub use otc_markets_order::{OtcMarketsOrder, OtcMarketsOrderRef};
pub use profile::{Profile, ProfileRef};
pub use quote::{Quote, QuoteRef};
pub use series::{Series, SeriesRef};
pub use source::{IndexedEventSource, OrderSource};
pub use spread_order::{SpreadOrder, SpreadOrderRef};
pub use summary::{Summary, SummaryRef};
pub use text_configuration::{TextConfiguration, TextConfigurationRef};
pub use text_message::{TextMessage, TextMessageRef};
pub use theo_price::{TheoPrice, TheoPriceRef};
pub use time_and_sale::{TimeAndSale, TimeAndSaleRef};
pub use trade::{Trade, TradeRef};
pub use trade_eth::{TradeETH, TradeETHRef};
use traits::AsDyn;
pub use traits::{BaseEvent, IndexedEvent, LastingEvent, MarketEvent, OrderEvent, TimeSeriesEvent};
pub use underlying::{Underlying, UnderlyingRef};

use crate::error::{Error, Result, c_string};

/// What the table of an event type generates for the code of this module.
trait Table {
    /// The SDK struct of the event.
    type Sys;
    /// The SDK class of the event.
    const CLASS: sys::dxfg_event_clazz_t;

    /// The SDK struct of the event; its strings point into `strings`, which must outlive it.
    fn to_sys(&self, strings: &mut Vec<CString>) -> Result<Self::Sys>;
}

/// Generates what each event type adds to the enums and traits of this module, from the list of types: the name of the
/// owned type, its view and the traits of [`traits`] it implements.
macro_rules! event_types {
    ($( $(#[$doc:meta])* $name:ident($view:ident) [$($trait_:ident)*], )*) => {
        /// An event type a subscription can receive.
        ///
        /// It prints as and parses from the simple name of its Java class, such as `Quote` or `TimeAndSale`:
        /// `"Quote".parse::<EventType>()`.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #[non_exhaustive]
        pub enum EventType {
            $( $(#[$doc])* $name, )*
        }

        impl EventType {
            /// Every event type.
            pub const ALL: &'static [EventType] = &[ $( EventType::$name, )* ];

            /// The simple name of the Java class of the event type, such as `Quote` or `TimeAndSale`.
            pub fn name(self) -> &'static str {
                match self {
                    $( EventType::$name => stringify!($name), )*
                }
            }

            pub(crate) fn to_sys(self) -> sys::dxfg_event_clazz_t {
                match self {
                    $( EventType::$name => <$name as Table>::CLASS, )*
                }
            }
        }

        impl Display for EventType {
            fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
                f.pad(self.name())
            }
        }

        impl std::str::FromStr for EventType {
            type Err = Error;

            /// Parses the simple name of the Java class of an event type; the case matters.
            ///
            /// Returns [`Error::InvalidArgument`] for any other string.
            fn from_str(name: &str) -> Result<Self> {
                match name {
                    $( stringify!($name) => Ok(EventType::$name), )*
                    _ => Err(Error::InvalidArgument(format!("unknown event type {name:?}"))),
                }
            }
        }

        /// An event received by a listener, valid during the callback only.
        #[derive(Debug, Clone, Copy)]
        #[non_exhaustive]
        pub enum EventRef<'a> {
            $( $(#[$doc])* $name(&'a $view), )*
            /// An event of a type this crate does not map, with its SDK class code (`dxfg_event_clazz_t`).
            Other(i32),
        }

        /// The accessors of every event, as Java's `EventType`, `IndexedEvent` and `TimeSeriesEvent` have them. Each
        /// returns `None` for [`EventRef::Other`], whose layout this crate does not know; the indexed and time-series
        /// values are also `None` for the types without them.
        impl<'a> EventRef<'a> {
            /// Copies the event, or returns `None` for [`EventRef::Other`].
            pub fn to_owned(&self) -> Option<Event> {
                match *self {
                    $( EventRef::$name(e) => Some(Event::$name(e.to_owned())), )*
                    EventRef::Other(_) => None,
                }
            }

            /// The symbol; replacement characters stand for bytes that are not UTF-8.
            #[doc(alias = "getEventSymbol")]
            pub fn symbol(&self) -> Option<Cow<'a, str>> {
                match *self {
                    $( EventRef::$name(e) => Some(e.symbol()), )*
                    EventRef::Other(_) => None,
                }
            }

            /// The time the event was created, in milliseconds since the Unix epoch; `0` if unknown.
            #[doc(alias = "getEventTime")]
            pub fn event_time(&self) -> Option<i64> {
                match *self {
                    $( EventRef::$name(e) => Some(e.event_time()), )*
                    EventRef::Other(_) => None,
                }
            }

            /// The type of the event.
            pub fn event_type(&self) -> Option<EventType> {
                match *self {
                    $( EventRef::$name(_) => Some(EventType::$name), )*
                    EventRef::Other(_) => None,
                }
            }

            /// The index of an [`IndexedEvent`].
            #[doc(alias = "getIndex")]
            pub fn index(&self) -> Option<i64> {
                self.as_indexed_event().map(IndexedEvent::index)
            }

            /// The flags of an [`IndexedEvent`].
            #[doc(alias = "getEventFlags")]
            pub fn event_flags(&self) -> Option<EventFlags> {
                self.as_indexed_event().map(IndexedEvent::event_flags)
            }

            /// The source id of an [`IndexedEvent`].
            #[doc(alias = "getSource")]
            pub fn source(&self) -> Option<i32> {
                self.as_indexed_event().map(IndexedEvent::source)
            }

            /// The time of a [`TimeSeriesEvent`], in milliseconds since the Unix epoch.
            #[doc(alias = "getTime")]
            pub fn time(&self) -> Option<i64> {
                self.as_time_series_event().map(TimeSeriesEvent::time)
            }

            /// The event as a [`MarketEvent`].
            pub fn as_market_event(&self) -> Option<&'a dyn MarketEvent> {
                match *self {
                    $( EventRef::$name(e) => e.as_market_event(), )*
                    EventRef::Other(_) => None,
                }
            }

            /// The event as a [`LastingEvent`].
            pub fn as_lasting_event(&self) -> Option<&'a dyn LastingEvent> {
                match *self {
                    $( EventRef::$name(e) => e.as_lasting_event(), )*
                    EventRef::Other(_) => None,
                }
            }

            /// The event as an [`IndexedEvent`].
            pub fn as_indexed_event(&self) -> Option<&'a dyn IndexedEvent> {
                match *self {
                    $( EventRef::$name(e) => e.as_indexed_event(), )*
                    EventRef::Other(_) => None,
                }
            }

            /// The event as a [`TimeSeriesEvent`].
            pub fn as_time_series_event(&self) -> Option<&'a dyn TimeSeriesEvent> {
                match *self {
                    $( EventRef::$name(e) => e.as_time_series_event(), )*
                    EventRef::Other(_) => None,
                }
            }

            /// The event as an [`OrderEvent`]: an order of any type.
            pub fn as_order_event(&self) -> Option<&'a dyn OrderEvent> {
                match *self {
                    $( EventRef::$name(e) => e.as_order_event(), )*
                    EventRef::Other(_) => None,
                }
            }

            /// The view of the event if it is of type `E`: `event.downcast::<Quote>()` gives a `&QuoteRef`.
            pub fn downcast<E: EventKind>(&self) -> Option<E::Item<'a>> {
                E::from_ref(*self)
            }
        }

        /// The text of Java's `toString` of the event; an event of an unmapped type prints as `Event{class=<code>}`.
        impl Display for EventRef<'_> {
            fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
                match self {
                    $( EventRef::$name(e) => e.fmt(f), )*
                    EventRef::Other(class) => write!(f, "Event{{class={class}}}"),
                }
            }
        }

        /// An owned event, e.g. for publishing.
        #[derive(Debug, Clone, PartialEq)]
        #[non_exhaustive]
        pub enum Event {
            $( $(#[$doc])* $name($name), )*
        }

        /// The accessors of every event, as Java's `EventType`, `IndexedEvent` and `TimeSeriesEvent` have them; the
        /// indexed and time-series values are `None` for the types without them.
        impl Event {
            /// The symbol.
            #[doc(alias = "getEventSymbol")]
            pub fn symbol(&self) -> &str {
                match self {
                    $( Event::$name(e) => &e.symbol, )*
                }
            }

            /// Changes the symbol.
            #[doc(alias = "setEventSymbol")]
            pub fn set_symbol(&mut self, symbol: impl Into<String>) {
                let symbol = symbol.into();
                match self {
                    $( Event::$name(e) => e.symbol = symbol, )*
                }
            }

            /// The time the event was created, in milliseconds since the Unix epoch; `0` if unknown.
            #[doc(alias = "getEventTime")]
            pub fn event_time(&self) -> i64 {
                match self {
                    $( Event::$name(e) => e.event_time, )*
                }
            }

            /// Changes the time the event was created, in milliseconds since the Unix epoch.
            #[doc(alias = "setEventTime")]
            pub fn set_event_time(&mut self, event_time: i64) {
                match self {
                    $( Event::$name(e) => e.event_time = event_time, )*
                }
            }

            /// The type of the event.
            pub fn event_type(&self) -> EventType {
                match self {
                    $( Event::$name(_) => EventType::$name, )*
                }
            }

            /// The index of an [`IndexedEvent`].
            #[doc(alias = "getIndex")]
            pub fn index(&self) -> Option<i64> {
                self.as_indexed_event().map(IndexedEvent::index)
            }

            /// The flags of an [`IndexedEvent`].
            #[doc(alias = "getEventFlags")]
            pub fn event_flags(&self) -> Option<EventFlags> {
                self.as_indexed_event().map(IndexedEvent::event_flags)
            }

            /// The source id of an [`IndexedEvent`].
            #[doc(alias = "getSource")]
            pub fn source(&self) -> Option<i32> {
                self.as_indexed_event().map(IndexedEvent::source)
            }

            /// The time of a [`TimeSeriesEvent`], in milliseconds since the Unix epoch.
            #[doc(alias = "getTime")]
            pub fn time(&self) -> Option<i64> {
                self.as_time_series_event().map(TimeSeriesEvent::time)
            }

            /// The event as a [`MarketEvent`].
            pub fn as_market_event(&self) -> Option<&dyn MarketEvent> {
                match self {
                    $( Event::$name(e) => e.as_market_event(), )*
                }
            }

            /// The event as a [`LastingEvent`].
            pub fn as_lasting_event(&self) -> Option<&dyn LastingEvent> {
                match self {
                    $( Event::$name(e) => e.as_lasting_event(), )*
                }
            }

            /// The event as an [`IndexedEvent`].
            pub fn as_indexed_event(&self) -> Option<&dyn IndexedEvent> {
                match self {
                    $( Event::$name(e) => e.as_indexed_event(), )*
                }
            }

            /// The event as a [`TimeSeriesEvent`].
            pub fn as_time_series_event(&self) -> Option<&dyn TimeSeriesEvent> {
                match self {
                    $( Event::$name(e) => e.as_time_series_event(), )*
                }
            }

            /// The event as an [`OrderEvent`]: an order of any type.
            pub fn as_order_event(&self) -> Option<&dyn OrderEvent> {
                match self {
                    $( Event::$name(e) => e.as_order_event(), )*
                }
            }

            /// The event if it is of type `E`: `event.downcast_ref::<Quote>()` gives a `&Quote`.
            pub fn downcast_ref<E: EventKind>(&self) -> Option<&E> {
                E::from_event(self)
            }
        }

        impl traits::sealed::Sealed for Event {}

        impl BaseEvent for Event {
            fn symbol(&self) -> Cow<'_, str> {
                Cow::Borrowed(Event::symbol(self))
            }

            fn event_time(&self) -> i64 {
                Event::event_time(self)
            }

            fn event_type(&self) -> EventType {
                Event::event_type(self)
            }
        }

        /// The text of Java's `toString` of the event.
        impl Display for Event {
            fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
                match self {
                    $( Event::$name(e) => e.fmt(f), )*
                }
            }
        }

        $(
            impl From<$name> for Event {
                fn from(event: $name) -> Self {
                    Event::$name(event)
                }
            }

            /// The event if it is of this type; otherwise the error gives the event back.
            impl TryFrom<Event> for $name {
                type Error = Event;

                fn try_from(event: Event) -> std::result::Result<Self, Event> {
                    match event {
                        Event::$name(e) => Ok(e),
                        other => Err(other),
                    }
                }
            }

            impl sealed::Sealed for $name {
                const ALL: bool = false;

                unsafe fn item<'a>(event: *mut sys::dxfg_event_type_t) -> Option<&'a $view> {
                    // SAFETY: the caller guarantees the event is live for `'a`; an event of the class of the type is
                    // its SDK struct, which the view wraps.
                    unsafe { ((*event).clazz == <$name as Table>::CLASS).then(|| &*event.cast::<$view>()) }
                }

                fn from_ref(event: EventRef<'_>) -> Option<&$view> {
                    match event {
                        EventRef::$name(e) => Some(e),
                        _ => None,
                    }
                }

                fn from_event(event: &Event) -> Option<&Self> {
                    match event {
                        Event::$name(e) => Some(e),
                        _ => None,
                    }
                }
            }

            impl SubscriptionItem for $name {
                type Item<'a> = &'a $view;
            }

            impl EventKind for $name {
                const TYPE: EventType = EventType::$name;
            }

            event_traits!(owned $name, $name, [$($trait_)*]);
            event_traits!(view $view, $name, [$($trait_)*]);
        )*

        impl sealed::Sealed for Event {
            const ALL: bool = true;

            unsafe fn item<'a>(event: *mut sys::dxfg_event_type_t) -> Option<EventRef<'a>> {
                // SAFETY: the caller guarantees the event is live for `'a`; its first field is the class, and an event
                // of a class is its SDK struct, which the view wraps.
                Some(unsafe {
                    match (*event).clazz {
                        $( <$name as Table>::CLASS => EventRef::$name(&*event.cast::<$view>()), )*
                        other => EventRef::Other(other),
                    }
                })
            }

            fn from_ref(event: EventRef<'_>) -> Option<EventRef<'_>> {
                Some(event)
            }

            fn from_event(event: &Event) -> Option<&Self> {
                Some(event)
            }
        }

        impl SubscriptionItem for Event {
            type Item<'a> = EventRef<'a>;
        }

        /// The SDK struct of an owned event.
        enum SysEvent {
            $( $name(<$name as Table>::Sys), )*
        }

        impl SysEvent {
            fn new(event: &Event, strings: &mut Vec<CString>) -> Result<Self> {
                Ok(match event {
                    $( Event::$name(e) => SysEvent::$name(e.to_sys(strings)?), )*
                })
            }

            fn as_mut_ptr(&mut self) -> *mut sys::dxfg_event_type_t {
                match self {
                    $( SysEvent::$name(e) => (e as *mut <$name as Table>::Sys).cast(), )*
                }
            }
        }
    };
}

event_types! {
    /// [`Quote`]: the best bid and offer.
    Quote(QuoteRef) [MarketEvent LastingEvent],
    /// [`Profile`]: the description and trading limits of a symbol.
    Profile(ProfileRef) [MarketEvent LastingEvent],
    /// [`Summary`]: the prices and volume of the day.
    Summary(SummaryRef) [MarketEvent LastingEvent],
    /// [`Greeks`]: the price sensitivities of an option.
    Greeks(GreeksRef) [MarketEvent LastingEvent IndexedEvent TimeSeriesEvent],
    /// [`Candle`]: the prices and volume of a period.
    Candle(CandleRef) [LastingEvent IndexedEvent TimeSeriesEvent],
    /// [`DailyCandle`]: the prices and volume of a day.
    DailyCandle(DailyCandleRef) [LastingEvent IndexedEvent TimeSeriesEvent],
    /// [`Underlying`]: the volatility and option volumes of an underlying symbol.
    Underlying(UnderlyingRef) [MarketEvent LastingEvent IndexedEvent TimeSeriesEvent],
    /// [`TheoPrice`]: the theoretical price of an option.
    TheoPrice(TheoPriceRef) [MarketEvent LastingEvent IndexedEvent TimeSeriesEvent],
    /// [`Trade`]: the last trade.
    Trade(TradeRef) [MarketEvent LastingEvent],
    /// [`TradeETH`]: the last trade in extended trading hours.
    TradeETH(TradeETHRef) [MarketEvent LastingEvent],
    /// [`Configuration`]: a configuration with a version and an attachment.
    Configuration(ConfigurationRef) [LastingEvent],
    /// [`Message`]: a message with an attachment.
    Message(MessageRef) [],
    /// [`TimeAndSale`]: a trade or another market event with a price.
    TimeAndSale(TimeAndSaleRef) [MarketEvent IndexedEvent TimeSeriesEvent],
    /// [`OrderBase`]: the fields that all the orders share.
    OrderBase(OrderBaseRef) [MarketEvent IndexedEvent OrderEvent],
    /// [`Order`]: an order or a price level of an order book.
    Order(OrderRef) [MarketEvent IndexedEvent OrderEvent],
    /// [`AnalyticOrder`]: an order with the analytics of its iceberg part.
    AnalyticOrder(AnalyticOrderRef) [MarketEvent IndexedEvent OrderEvent],
    /// [`OtcMarketsOrder`]: an order of OTC Markets.
    OtcMarketsOrder(OtcMarketsOrderRef) [MarketEvent IndexedEvent OrderEvent],
    /// [`SpreadOrder`]: an order or a price level of an order book for a spread.
    SpreadOrder(SpreadOrderRef) [MarketEvent IndexedEvent OrderEvent],
    /// [`Series`]: the properties of an option series.
    Series(SeriesRef) [MarketEvent IndexedEvent],
    /// [`OptionSale`]: a trade or another market event with a price for an option.
    OptionSale(OptionSaleRef) [MarketEvent IndexedEvent],
    /// [`TextMessage`]: a text message with a time.
    TextMessage(TextMessageRef) [],
    /// [`MarketMaker`]: the best bid and ask of a market maker.
    MarketMaker(MarketMakerRef) [MarketEvent IndexedEvent],
    /// [`TextConfiguration`]: a configuration as text with a time and a version.
    TextConfiguration(TextConfigurationRef) [LastingEvent],
    /// [`NuamOrder`]: an order of the Nuam exchange.
    NuamOrder(NuamOrderRef) [MarketEvent IndexedEvent OrderEvent],
    /// [`NuamTimeAndSale`]: a time and sale of the Nuam exchange.
    NuamTimeAndSale(NuamTimeAndSaleRef) [MarketEvent IndexedEvent TimeSeriesEvent],
    /// [`NuamTrade`]: the last trade of the Nuam exchange.
    NuamTrade(NuamTradeRef) [MarketEvent LastingEvent],
    /// [`OrderImbalance`]: the imbalance of the orders before an auction.
    OrderImbalance(OrderImbalanceRef) [MarketEvent],
}

/// What a [`DXFeedSubscription`](crate::DXFeedSubscription) delivers: an owned event type such as [`Quote`] for a
/// subscription to one event type, [`Event`] for a subscription to several.
///
/// The trait is sealed: only this crate implements it.
pub trait SubscriptionItem: sealed::Sealed + Send + 'static {
    /// The item a listener iterates: a view such as `&QuoteRef`, or [`EventRef`].
    type Item<'a>;
}

/// An owned event type such as [`Quote`]: what only owned events can do (clone, convert to and from [`Event`]) on top
/// of the [`BaseEvent`] accessors that views share. A subscription is created for it by type, see
/// [`DXFeed::create_subscription`](crate::DXFeed::create_subscription).
///
/// The trait is sealed: only this crate implements it.
pub trait EventKind: SubscriptionItem + BaseEvent + Clone + Into<Event> + TryFrom<Event, Error = Event> {
    /// The event type.
    const TYPE: EventType;
}

mod sealed {
    use super::{Event, EventRef, SubscriptionItem, sys};

    pub trait Sealed {
        /// Whether every SDK event converts to an item, so that [`Events`](super::Events) knows its exact length.
        const ALL: bool;

        /// Converts an SDK event, or returns `None` for an event of another type.
        ///
        /// # Safety
        ///
        /// `event` must be a live SDK event that outlives `'a`.
        unsafe fn item<'a>(event: *mut sys::dxfg_event_type_t) -> Option<Self::Item<'a>>
        where
            Self: SubscriptionItem;

        /// The item of an event a listener received, or `None` for an event of another type.
        fn from_ref(event: EventRef<'_>) -> Option<Self::Item<'_>>
        where
            Self: SubscriptionItem;

        /// The owned event, or `None` for an event of another type.
        fn from_event(event: &Event) -> Option<&Self>;
    }
}

/// The events of one listener call, borrowed from the SDK: a view such as `&QuoteRef` for a subscription to one event
/// type, [`EventRef`] for an untyped one.
///
/// A typed iterator skips events of other types; the SDK does not deliver them.
pub struct Events<'a, E: SubscriptionItem = Event> {
    elements: &'a [*mut sys::dxfg_event_type_t],
    next: usize,
    _list: PhantomData<&'a sys::dxfg_event_type_list>,
    _item: PhantomData<fn() -> E>,
}

impl<'a> Events<'a> {
    /// # Safety
    ///
    /// `list` must be NULL or a live SDK event list that outlives `'a`.
    pub(crate) unsafe fn new(list: *const sys::dxfg_event_type_list) -> Self {
        // SAFETY: the caller guarantees `list` is NULL or live for `'a`.
        let elements = match unsafe { list.as_ref() } {
            // SAFETY: a live list holds `size` element pointers.
            Some(list) if list.size > 0 && !list.elements.is_null() => unsafe {
                std::slice::from_raw_parts(list.elements, list.size as usize)
            },
            _ => &[],
        };
        Events {
            elements,
            next: 0,
            _list: PhantomData,
            _item: PhantomData,
        }
    }

    /// The same events as items of `E`.
    pub(crate) fn typed<E: SubscriptionItem>(self) -> Events<'a, E> {
        Events {
            elements: self.elements,
            next: self.next,
            _list: PhantomData,
            _item: PhantomData,
        }
    }
}

impl<'a, E: SubscriptionItem> Iterator for Events<'a, E> {
    type Item = E::Item<'a>;

    fn next(&mut self) -> Option<E::Item<'a>> {
        while let Some(&event) = self.elements.get(self.next) {
            self.next += 1;
            // SAFETY: the elements of a live list are live events for `'a`.
            if let Some(item) = unsafe { E::item(event) } {
                return Some(item);
            }
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.elements.len() - self.next;
        (if E::ALL { n } else { 0 }, Some(n))
    }
}

impl ExactSizeIterator for Events<'_, Event> {}

/// A Java `char` exchange code from the SDK.
fn exchange_code(code: i16) -> char {
    char::from_u32(code as u16 as u32).unwrap_or(char::REPLACEMENT_CHARACTER)
}

/// A Java `char` exchange code for the SDK; it must fit 16 bits.
fn sys_exchange_code(code: char) -> Result<i16> {
    u16::try_from(code as u32)
        .map(|c| c as i16)
        .map_err(|_| Error::InvalidArgument(format!("exchange code {code:?} does not fit a Java char")))
}

/// A nullable string of an SDK event; replacement characters stand for bytes that are not UTF-8.
///
/// # Safety
///
/// `s` must be NULL or a NUL-terminated string that outlives `'a`.
unsafe fn opt_string<'a>(s: *const c_char) -> Option<Cow<'a, str>> {
    // SAFETY: the caller guarantees a NUL-terminated string.
    (!s.is_null()).then(|| unsafe { CStr::from_ptr(s) }.to_string_lossy())
}

/// Pattern `p` of the strings of the extreme-value tests: NULL, bytes that are not UTF-8, characters outside ASCII.
#[cfg(test)]
fn extreme_string(strings: &mut Vec<CString>, p: usize) -> *const c_char {
    let bytes: &[u8] = match p % 3 {
        0 => return ptr::null(),
        1 => b"\xff\xfe",
        _ => "\u{1F600} \u{e9}".as_bytes(),
    };
    let s = CString::new(bytes).unwrap();
    let pointer = s.as_ptr();
    strings.push(s);
    pointer
}

/// Keeps a NUL-terminated copy of `s` in `strings` and returns a pointer to it, or NULL for `None`; `what` names the
/// string in the error for a NUL byte.
fn arena_string(strings: &mut Vec<CString>, s: Option<&str>, what: &str) -> Result<*const c_char> {
    let Some(s) = s else {
        return Ok(ptr::null());
    };
    let s = c_string(s, what)?;
    // The heap buffer of a `CString` does not move when `strings` grows.
    let pointer = s.as_ptr();
    strings.push(s);
    Ok(pointer)
}

/// SDK events built from owned ones, with the strings they point to; kept alive while the SDK reads them.
pub(crate) struct SysEvents {
    events: Vec<SysEvent>,
    _strings: Vec<CString>,
    pointers: Vec<*mut sys::dxfg_event_type_t>,
}

impl SysEvents {
    pub(crate) fn new(events: &[Event]) -> Result<Self> {
        let mut strings = Vec::new();
        let mut sys_events = events
            .iter()
            .map(|event| SysEvent::new(event, &mut strings))
            .collect::<Result<Vec<_>>>()?;
        let pointers = sys_events.iter_mut().map(SysEvent::as_mut_ptr).collect();
        Ok(SysEvents {
            events: sys_events,
            _strings: strings,
            pointers,
        })
    }

    /// The SDK events, valid while `self` is neither moved nor dropped.
    pub(crate) fn pointers(&self) -> &[*mut sys::dxfg_event_type_t] {
        &self.pointers
    }

    /// The SDK list over the events, valid while `self` is neither moved nor dropped.
    pub(crate) fn list(&mut self) -> sys::dxfg_event_type_list {
        debug_assert_eq!(self.events.len(), self.pointers.len());
        sys::dxfg_event_type_list {
            size: self.pointers.len() as i32,
            elements: self.pointers.as_mut_ptr(),
        }
    }
}
