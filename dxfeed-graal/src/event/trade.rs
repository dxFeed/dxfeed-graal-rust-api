//! Trade events.

use dxfeed_graal_sys as sys;

use super::format::{JavaChar, JavaDouble, Time, year_month_day};

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
        write!(
            f,
            "Trade{{{}, eventTime={}, time={}, timeNanoPart={}, sequence={}, exchange={}, tradeId={}, price={}, \
             change={}, size={}, day={}, dayVolume={}, dayTurnover={}, direction={}, ETH={}}}",
            self.symbol,
            Time(self.event_time),
            Time(self.time()),
            self.time_nano_part,
            self.sequence(),
            JavaChar(self.exchange_code),
            self.trade_id,
            JavaDouble(self.price),
            JavaDouble(self.change),
            JavaDouble(self.size),
            year_month_day(self.day_id),
            JavaDouble(self.day_volume),
            JavaDouble(self.day_turnover),
            self.tick_direction(),
            self.is_extended_trading_hours(),
        )
    }
}
