//! The behavior several event types share, as the Java interfaces of the same names.

use super::{Event, EventFlags, EventKind, Scope, Side};

/// An event about a market symbol, as Java's `MarketEvent`.
///
/// The trait is sealed: only this crate implements it.
pub trait MarketEvent: EventKind {
    /// The symbol.
    #[doc(alias = "getEventSymbol")]
    fn symbol(&self) -> &str;

    /// The time the event was created, in milliseconds since the Unix epoch; `0` if unknown.
    #[doc(alias = "getEventTime")]
    fn event_time(&self) -> i64;
}

/// An event whose last value a feed keeps for each symbol, as Java's `LastingEvent`: a new event replaces the previous
/// one. A feed returns the last value with
/// [`DXFeed::get_last_event_if_subscribed`](crate::DXFeed::get_last_event_if_subscribed),
/// [`DXFeed::get_last_event`](crate::DXFeed::get_last_event) and
/// [`DXFeed::get_last_event_promise`](crate::DXFeed::get_last_event_promise).
///
/// The trait is sealed: only this crate implements it.
pub trait LastingEvent: EventKind + Clone + Into<Event> + TryFrom<Event, Error = Event> {}

/// An event that a source and an index tell apart from the other events of its symbol, as Java's `IndexedEvent`.
///
/// The trait is sealed: only this crate implements it.
pub trait IndexedEvent: EventKind + Clone + Into<Event> + TryFrom<Event, Error = Event> {
    /// The index of the event among the events of its symbol and source.
    #[doc(alias = "getIndex")]
    fn index(&self) -> i64;

    /// The flags of the event in a transaction or a snapshot.
    #[doc(alias = "getEventFlags")]
    fn event_flags(&self) -> EventFlags;

    /// The id of the source of the event: `0` (`DEFAULT`) for all but the orders, whose index keeps it;
    /// [`IndexedEventSource::from_id`](super::IndexedEventSource::from_id) gives the source.
    #[doc(alias = "getSource")]
    fn source(&self) -> i32;
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
pub trait OrderEvent: IndexedEvent + Sync {
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
    fn market_maker(&self) -> Option<&str>;
}

/// Implements [`OrderEvent`] for the order types, each with the expression of its market maker.
macro_rules! order_event {
    ($($name:ident: |$order:ident| $market_maker:expr;)*) => {$(
        impl OrderEvent for super::$name {
            fn price(&self) -> f64 {
                self.price
            }

            fn size(&self) -> f64 {
                self.size
            }

            fn time_sequence(&self) -> i64 {
                self.time_sequence
            }

            fn exchange_code(&self) -> char {
                super::$name::exchange_code(self)
            }

            fn order_side(&self) -> Side {
                super::$name::order_side(self)
            }

            fn scope(&self) -> Scope {
                super::$name::scope(self)
            }

            fn market_maker(&self) -> Option<&str> {
                let $order = self;
                $market_maker
            }
        }
    )*};
}

order_event! {
    OrderBase: |_order| None;
    Order: |order| order.market_maker.as_deref();
    AnalyticOrder: |order| order.market_maker.as_deref();
    OtcMarketsOrder: |order| order.market_maker.as_deref();
    NuamOrder: |order| order.market_maker.as_deref();
    SpreadOrder: |_order| None;
}

/// Implements a trait of this module for an event type, from the fields its table generates.
macro_rules! impl_event_trait {
    (MarketEvent, $name:ident) => {
        impl MarketEvent for $name {
            fn symbol(&self) -> &str {
                &self.symbol
            }

            fn event_time(&self) -> i64 {
                self.event_time
            }
        }
    };
    (LastingEvent, $name:ident) => {
        impl LastingEvent for $name {}
    };
    (IndexedEvent, $name:ident) => {
        impl IndexedEvent for $name {
            fn index(&self) -> i64 {
                self.index
            }

            fn event_flags(&self) -> EventFlags {
                self.event_flags
            }

            fn source(&self) -> i32 {
                $name::source(self)
            }
        }
    };
    (TimeSeriesEvent, $name:ident) => {
        impl TimeSeriesEvent for $name {
            fn time(&self) -> i64 {
                $name::time(self)
            }
        }
    };
}
