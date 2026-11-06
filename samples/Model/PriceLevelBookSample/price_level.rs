// The module keeps all the methods of the C++ header; the sample uses some of them.
#![allow(dead_code)]

use std::fmt;

use dxfeed_graal::event::{IndexedEventSource, Order, Side};

/// Represents a price level in a price level book.
#[derive(Clone, Debug)]
pub struct PriceLevel {
    event_symbol: Option<String>,
    event_source: IndexedEventSource,
    side: Side,
    price: f64,
    size: f64,
}

impl Default for PriceLevel {
    /// Initializes a new default instance.
    fn default() -> Self {
        PriceLevel {
            event_symbol: None,
            event_source: IndexedEventSource::DEFAULT,
            side: Side::Undefined,
            price: 0.0,
            size: 0.0,
        }
    }
}

impl PriceLevel {
    /// Initializes a new instance using an order.
    ///
    /// `order` is the order to initialize the price level from.
    pub fn new(order: &Order) -> Self {
        PriceLevel {
            event_symbol: Some(order.symbol.clone()),
            event_source: order.source(),
            side: order.order_side(),
            price: order.price,
            size: order.size,
        }
    }

    /// Returns the optional event symbol.
    pub fn get_event_symbol(&self) -> Option<&str> {
        self.event_symbol.as_deref()
    }

    /// Returns the event source.
    pub fn get_event_source(&self) -> IndexedEventSource {
        self.event_source
    }

    /// Returns the side of the price level.
    pub fn get_side(&self) -> Side {
        self.side
    }

    /// Returns the price of the price level.
    pub fn get_price(&self) -> f64 {
        self.price
    }

    /// Returns the size of the price level.
    pub fn get_size(&self) -> f64 {
        self.size
    }

    /// Sets the size of the price level.
    ///
    /// `size` is the new price level size.
    pub fn set_size(&mut self, size: f64) {
        self.size = size;
    }

    /// Compares two price levels without taking price into account (`operator==` of C++).
    ///
    /// Returns `true` if the event symbols are not `None`, are equal, and the sides are equal.
    pub fn equals(pl1: &PriceLevel, pl2: &PriceLevel) -> bool {
        if pl1.get_event_symbol().is_none() || pl2.get_event_symbol().is_none() {
            return false;
        }

        if pl1.get_event_symbol() != pl2.get_event_symbol() {
            return false;
        }

        pl1.get_side() == pl2.get_side()
    }
}

/// Returns a string representation of this price level.
impl fmt::Display for PriceLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "PriceLevel{{{}, source={}, side={}, price={:?}, size={:?}}}",
            self.event_symbol.as_deref().unwrap_or("null"),
            self.event_source,
            self.side,
            self.price,
            self.size
        )
    }
}
