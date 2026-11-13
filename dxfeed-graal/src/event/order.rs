//! Order events.

use dxfeed_graal_sys as sys;

use super::format::Quoted;

order_base_fields! {
    [order_base]
    event_type! {
        head {
            /// An order or a price level of an order book (`Order`). The [`source`](Self::source) tells the books
            /// apart, and the [`scope`](Self::scope) tells an individual order from an aggregate price level.
            ///
            /// A subscription to orders also receives the price levels that the feed builds from quotes (the
            /// sources `COMPOSITE_BID` and `COMPOSITE_ASK`) and from market makers (`AGGREGATE_BID` and
            /// `AGGREGATE_ASK`).
            ///
            /// # Examples
            ///
            /// The orders of `AAPL` from `NTV` only, with their side, scope and price:
            ///
            /// ```no_run
            /// # fn main() -> dxfeed_graal::Result<()> {
            /// use dxfeed_graal::event::{Order, OrderSource};
            /// use dxfeed_graal::{DXFeed, Symbol};
            ///
            /// let subscription = DXFeed::get_instance()?.create_subscription::<Order>()?;
            /// subscription.add_event_listener(|orders| {
            ///     for order in orders {
            ///         let (side, scope) = (order.order_side(), order.scope());
            ///         println!("{} {side} {scope}: {} x {}", order.index(), order.price(), order.size());
            ///     }
            /// })?;
            /// subscription.add_symbols([Symbol::indexed("AAPL", OrderSource::NTV)])?;
            /// # Ok(())
            /// # }
            /// ```
            ///
            /// The events of an order book come in snapshots and transactions (see [`EventFlags`](super::EventFlags));
            /// [`OrderBookModel`](crate::model::OrderBookModel) and
            /// [`MarketDepthModel`](crate::model::MarketDepthModel) keep the book itself.
            Order, OrderRef, sys::dxfg_order_t, sys::DXFG_EVENT_ORDER, [order_base.market_event]
        }
        fields {
            /// The identifier of the market maker or of another aggregate of the order; `None` if unknown.
            #[doc(alias = "getMarketMaker")]
            market_maker: string [market_maker] = None,
        }
        derived {}
    }
}

/// The text of Java's `Order.toString`, e.g. `Order{AAPL, eventTime=0, source=DEFAULT, eventFlags=0x0, ...}`.
impl std::fmt::Display for Order {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Order{")?;
        self.fmt_order_base(f)?;
        write!(f, ", marketMaker={}}}", Quoted(self.market_maker.as_deref()))
    }
}
