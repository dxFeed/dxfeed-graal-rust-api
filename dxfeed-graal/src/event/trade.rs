//! Trade events.

use dxfeed_graal_sys as sys;

trade_base_fields! {
    [trade_base]
    event_type! {
        head {
            /// A trade: the last trade of a symbol during regular trading hours or extended ones (`Trade`).
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
