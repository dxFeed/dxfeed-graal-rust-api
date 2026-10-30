//! Trade events of extended trading hours.

use dxfeed_graal_sys as sys;

trade_base_fields! {
    [trade_base]
    event_type! {
        head {
            /// A trade in extended trading hours: the last trade of a symbol outside regular trading hours
            /// (`TradeETH`).
            TradeETH, TradeETHRef, sys::dxfg_trade_eth_t, sys::DXFG_EVENT_TRADE_ETH, [trade_base.market_event]
        }
        fields {}
        derived {}
    }
}

/// The text of Java's `TradeETH.toString`, e.g. `TradeETH{AAPL, eventTime=0, time=20241004-180000.000, ...}`.
impl std::fmt::Display for TradeETH {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TradeETH{")?;
        self.fmt_trade_base(f)?;
        f.write_str("}")
    }
}
