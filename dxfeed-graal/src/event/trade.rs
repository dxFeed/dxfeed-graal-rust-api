//! Trade events.

use dxfeed_graal_sys as sys;

trade_base_fields! {
    [trade_base]
    event_type! {
        head {
            /// A trade: the last trade of a symbol during regular trading hours or extended ones (`Trade`).
            ///
            /// # Examples
            ///
            /// ```no_run
            /// # fn main() -> dxfeed_graal::Result<()> {
            /// use dxfeed_graal::DXFeed;
            /// use dxfeed_graal::event::Trade;
            ///
            /// let subscription = DXFeed::get_instance()?.create_subscription::<Trade>()?;
            /// subscription.add_event_listener(|trades| {
            ///     for trade in trades {
            ///         println!("{}: {} x {}", trade.symbol(), trade.price(), trade.size());
            ///         println!("day volume {}", trade.day_volume());
            ///     }
            /// })?;
            /// subscription.add_symbols(["AAPL", "IBM"])?;
            /// # Ok(())
            /// # }
            /// ```
            Trade, TradeRef, sys::dxfg_trade_t, sys::DXFG_EVENT_TRADE, [trade_base.market_event]
        }
        fields {}
        derived {}
    }
}

/// The text of Java's `Trade.toString`, e.g. `Trade{AAPL, eventTime=0, time=20241004-093000.000, ...}`; times print
/// in the default time zone of the SDK.
impl std::fmt::Display for Trade {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Trade{")?;
        self.fmt_trade_base(f)?;
        f.write_str("}")
    }
}
