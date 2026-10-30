//! The behavior several event types share, as the Java interfaces of the same names.

use super::EventKind;

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
/// one.
///
/// The trait is sealed: only this crate implements it.
pub trait LastingEvent: EventKind {}

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
}
