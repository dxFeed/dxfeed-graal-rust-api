//! Nuam trade events.

use dxfeed_graal_sys as sys;

use super::format::{JavaDouble, Time};

trade_base_fields! {
    [trade_base.trade_base]
    event_type! {
        head {
            /// A trade of the Nuam exchange, with its trading statistics (`NuamTrade`).
            NuamTrade, NuamTradeRef, sys::dxfg_nuam_trade_t, sys::DXFG_EVENT_NUAM_TRADE,
            [trade_base.trade_base.market_event]
        }
        fields {
            /// The time of the trading statistics, in milliseconds since the Unix epoch.
            trade_stat_time: i64 [trade_stat_time] = 0,
            /// The last price that changed the statistics; NaN if unknown.
            last_significant_price: f64 [last_significant_price] = f64::NAN,
            /// The last price of all the trades; NaN if unknown.
            last_price_for_all: f64 [last_price_for_all] = f64::NAN,
            /// The number of trades of the day.
            number_of_trades: i32 [number_of_trades] = 0,
            /// The volume-weighted average price; NaN if unknown.
            #[doc(alias = "getVWAP")]
            vwap: f64 [vwap] = f64::NAN,
        }
        derived {}
    }
}

/// The text of Java's `NuamTrade.toString`: the text of [`Trade`](super::Trade) with the statistics at the end.
impl std::fmt::Display for NuamTrade {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NuamTrade{")?;
        self.fmt_trade_base(f)?;
        write!(
            f,
            ", tradeStatTime={}, lastSignificantPrice={}, lastPriceForAll={}, numberOfTrades={}, vwap={}}}",
            Time(self.trade_stat_time),
            JavaDouble(self.last_significant_price),
            JavaDouble(self.last_price_for_all),
            self.number_of_trades,
            JavaDouble(self.vwap),
        )
    }
}
