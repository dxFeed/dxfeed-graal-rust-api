//! The behavior several event types share, as the Java interfaces of the same names.
//!
//! The traits only read: both the owned types, such as [`Quote`](super::Quote), and the views a listener receives,
//! such as [`QuoteRef`](super::QuoteRef), implement them, so one generic function serves both. What only an owned
//! type can do (clone, convert to and from [`Event`](super::Event)) is [`EventKind`](super::EventKind).

use std::borrow::Cow;

use super::{EventFlags, EventType, Scope, Side};

/// Any event, as Java's `EventType` interface (in this crate [`EventType`] names the enum of the event types): it
/// has a symbol and an event time.
///
/// The trait is sealed: only this crate implements it.
#[doc(alias = "EventType")]
pub trait BaseEvent: sealed::Sealed {
    /// The symbol; replacement characters stand for bytes of a received symbol that are not UTF-8.
    #[doc(alias = "getEventSymbol")]
    fn symbol(&self) -> Cow<'_, str>;

    /// The time the event was created, in milliseconds since the Unix epoch; `0` if unknown.
    #[doc(alias = "getEventTime")]
    fn event_time(&self) -> i64;

    /// The type of the event.
    fn event_type(&self) -> EventType;
}

/// An event about a market symbol, as Java's `MarketEvent`. Candles, messages and configurations are events but not
/// market events, as in Java.
///
/// The trait is sealed: only this crate implements it.
pub trait MarketEvent: BaseEvent {}

/// An event whose last value a feed keeps for each symbol, as Java's `LastingEvent`: a new event replaces the previous
/// one. A feed returns the last value with
/// [`DXFeed::get_last_event_if_subscribed`](crate::DXFeed::get_last_event_if_subscribed),
/// [`DXFeed::get_last_event`](crate::DXFeed::get_last_event) and
/// [`DXFeed::get_last_event_promise`](crate::DXFeed::get_last_event_promise).
///
/// The trait is sealed: only this crate implements it.
pub trait LastingEvent: BaseEvent {}

/// An event that a source and an index tell apart from the other events of its symbol, as Java's `IndexedEvent`.
///
/// The trait is sealed: only this crate implements it.
pub trait IndexedEvent: BaseEvent {
    /// The index of the event among the events of its symbol and source.
    #[doc(alias = "getIndex")]
    fn index(&self) -> i64;

    /// The flags of the event in a transaction or a snapshot.
    #[doc(alias = "getEventFlags")]
    fn event_flags(&self) -> EventFlags;

    /// The source of the event: [`DEFAULT`](super::IndexedEventSource::DEFAULT) for all but the orders, whose index
    /// keeps it.
    #[doc(alias = "getSource")]
    fn source(&self) -> super::IndexedEventSource;
}

/// An indexed event with a time, as Java's `TimeSeriesEvent`: the index of the event is its time and sequence.
///
/// The trait is sealed: only this crate implements it.
pub trait TimeSeriesEvent: IndexedEvent {
    /// The time of the event, in milliseconds since the Unix epoch.
    #[doc(alias = "getTime")]
    fn time(&self) -> i64;
}

/// An order: [`OrderBase`](super::OrderBase) or a type Java derives from it, such as [`Order`](super::Order). The
/// order book models, such as [`MarketDepthModel`](crate::model::MarketDepthModel), take it.
///
/// The trait is sealed: only this crate implements it.
pub trait OrderEvent: IndexedEvent {
    /// The price; NaN if unknown.
    #[doc(alias = "getPrice")]
    fn price(&self) -> f64;

    /// The size; NaN if unknown.
    #[doc(alias = "getSize")]
    fn size(&self) -> f64;

    /// Whether the order has a size: neither `0` nor NaN.
    #[doc(alias = "hasSize")]
    fn has_size(&self) -> bool {
        let size = self.size();
        size != 0.0 && !size.is_nan()
    }

    /// The time and the sequence of the order, packed.
    #[doc(alias = "getTimeSequence")]
    fn time_sequence(&self) -> i64;

    /// The exchange code; `'\0'` if unknown.
    #[doc(alias = "getExchangeCode")]
    fn exchange_code(&self) -> char;

    /// The side.
    #[doc(alias = "getOrderSide")]
    fn order_side(&self) -> Side;

    /// The scope.
    #[doc(alias = "getScope")]
    fn scope(&self) -> Scope;

    /// The market maker of an [`Order`](super::Order) and the types Java derives from it; `None` for the others and
    /// when unknown.
    #[doc(alias = "getMarketMaker")]
    fn market_maker(&self) -> Option<Cow<'_, str>>;
}

/// The traits of this module as trait objects, for the accessors of [`Event`](super::Event) and
/// [`EventRef`](super::EventRef): `None` where the type does not implement the trait.
pub(crate) trait AsDyn {
    fn as_market_event(&self) -> Option<&dyn MarketEvent>;
    fn as_lasting_event(&self) -> Option<&dyn LastingEvent>;
    fn as_indexed_event(&self) -> Option<&dyn IndexedEvent>;
    fn as_time_series_event(&self) -> Option<&dyn TimeSeriesEvent>;
    fn as_order_event(&self) -> Option<&dyn OrderEvent>;
}

pub(crate) mod sealed {
    pub trait Sealed {}
}

/// Reads a stored value of an owned event type (a field) or of a view (an accessor).
macro_rules! get {
    (owned, $e:expr, $f:ident) => {
        $e.$f
    };
    (view, $e:expr, $f:ident) => {
        $e.$f()
    };
}

/// Implements [`OrderEvent`] for the order types and their views; the market maker is `None` for the types without
/// it.
macro_rules! order_event {
    ($($name:ident $view:ident $mm:ident;)*) => {$(
        order_event!(@impl owned $name $mm);
        order_event!(@impl view $view $mm);
    )*};
    (@mm owned $order:ident yes) => { $order.market_maker.as_deref().map(Cow::Borrowed) };
    (@mm view $order:ident yes) => { $order.market_maker() };
    (@mm $kind:ident $order:ident no) => { None };
    (@impl $kind:ident $ty:ident $mm:ident) => {
        impl OrderEvent for super::$ty {
            fn price(&self) -> f64 {
                get!($kind, self, price)
            }

            fn size(&self) -> f64 {
                get!($kind, self, size)
            }

            fn time_sequence(&self) -> i64 {
                get!($kind, self, time_sequence)
            }

            fn exchange_code(&self) -> char {
                super::$ty::exchange_code(self)
            }

            fn order_side(&self) -> Side {
                super::$ty::order_side(self)
            }

            fn scope(&self) -> Scope {
                super::$ty::scope(self)
            }

            fn market_maker(&self) -> Option<Cow<'_, str>> {
                order_event!(@mm $kind self $mm)
            }
        }
    };
}

order_event! {
    OrderBase OrderBaseRef no;
    Order OrderRef yes;
    AnalyticOrder AnalyticOrderRef yes;
    OtcMarketsOrder OtcMarketsOrderRef yes;
    NuamOrder NuamOrderRef yes;
    SpreadOrder SpreadOrderRef no;
}

/// Implements a trait of this module for an owned event type or a view, from the accessors its table generates;
/// `OrderEvent` is implemented by [`order_event!`] and generates nothing here.
macro_rules! impl_event_trait {
    (MarketEvent, $kind:ident, $ty:ty) => {
        impl MarketEvent for $ty {}
    };
    (LastingEvent, $kind:ident, $ty:ty) => {
        impl LastingEvent for $ty {}
    };
    (IndexedEvent, $kind:ident, $ty:ty) => {
        impl IndexedEvent for $ty {
            fn index(&self) -> i64 {
                get!($kind, self, index)
            }

            fn event_flags(&self) -> EventFlags {
                get!($kind, self, event_flags)
            }

            fn source(&self) -> $crate::event::IndexedEventSource {
                <$ty>::source(self)
            }
        }
    };
    (TimeSeriesEvent, $kind:ident, $ty:ty) => {
        impl TimeSeriesEvent for $ty {
            fn time(&self) -> i64 {
                <$ty>::time(self)
            }
        }
    };
    (OrderEvent, $kind:ident, $ty:ty) => {};
}

/// `$yes` if the list of traits holds `$want`, else `$no`.
macro_rules! if_trait {
    ($want:ident [] $yes:block $no:block) => { $no };
    (MarketEvent [MarketEvent $($rest:ident)*] $yes:block $no:block) => { $yes };
    (LastingEvent [LastingEvent $($rest:ident)*] $yes:block $no:block) => { $yes };
    (IndexedEvent [IndexedEvent $($rest:ident)*] $yes:block $no:block) => { $yes };
    (TimeSeriesEvent [TimeSeriesEvent $($rest:ident)*] $yes:block $no:block) => { $yes };
    (OrderEvent [OrderEvent $($rest:ident)*] $yes:block $no:block) => { $yes };
    ($want:ident [$head:ident $($rest:ident)*] $yes:block $no:block) => { if_trait!($want [$($rest)*] $yes $no) };
}

/// Implements [`BaseEvent`], the traits of the list and [`AsDyn`] for an owned event type or a view; invoked in the
/// parent module.
macro_rules! event_traits {
    ($kind:ident $ty:ty, $name:ident, [$($trait_:ident)*]) => {
        impl traits::sealed::Sealed for $ty {}

        impl BaseEvent for $ty {
            fn symbol(&self) -> Cow<'_, str> {
                event_traits!(@symbol $kind self)
            }

            fn event_time(&self) -> i64 {
                get!($kind, self, event_time)
            }

            fn event_type(&self) -> EventType {
                EventType::$name
            }
        }

        $( impl_event_trait!($trait_, $kind, $ty); )*

        impl traits::AsDyn for $ty {
            fn as_market_event(&self) -> Option<&dyn MarketEvent> {
                if_trait!(MarketEvent [$($trait_)*] { Some(self) } { None })
            }

            fn as_lasting_event(&self) -> Option<&dyn LastingEvent> {
                if_trait!(LastingEvent [$($trait_)*] { Some(self) } { None })
            }

            fn as_indexed_event(&self) -> Option<&dyn IndexedEvent> {
                if_trait!(IndexedEvent [$($trait_)*] { Some(self) } { None })
            }

            fn as_time_series_event(&self) -> Option<&dyn TimeSeriesEvent> {
                if_trait!(TimeSeriesEvent [$($trait_)*] { Some(self) } { None })
            }

            fn as_order_event(&self) -> Option<&dyn OrderEvent> {
                if_trait!(OrderEvent [$($trait_)*] { Some(self) } { None })
            }
        }
    };
    (@symbol owned $e:ident) => { Cow::Borrowed($e.symbol.as_str()) };
    (@symbol view $e:ident) => { $e.symbol() };
}
