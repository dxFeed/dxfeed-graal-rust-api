//! Daily candle events.

use dxfeed_graal_sys as sys;

candle_fields! {
    [candle]
    event_type! {
        head {
            /// A daily candle: a [`Candle`](super::Candle) of a day (`DailyCandle`); Java deprecates it in favor of
            /// `Candle`.
            DailyCandle, DailyCandleRef, sys::dxfg_daily_candle_t, sys::DXFG_EVENT_DAILY_CANDLE, [candle]
        }
        fields {}
        derived {}
    }
}

/// Converts between [`Candle`](super::Candle) and [`DailyCandle`], which have the same fields: Java's `DailyCandle` is
/// a `Candle`.
macro_rules! convert {
    ($from:ident => $to:ident) => {
        impl From<super::$from> for super::$to {
            fn from(candle: super::$from) -> Self {
                super::$to {
                    symbol: candle.symbol,
                    event_time: candle.event_time,
                    event_flags: candle.event_flags,
                    index: candle.index,
                    count: candle.count,
                    open: candle.open,
                    high: candle.high,
                    low: candle.low,
                    close: candle.close,
                    volume: candle.volume,
                    vwap: candle.vwap,
                    bid_volume: candle.bid_volume,
                    ask_volume: candle.ask_volume,
                    imp_volatility: candle.imp_volatility,
                    open_interest: candle.open_interest,
                }
            }
        }
    };
}

convert!(DailyCandle => Candle);
convert!(Candle => DailyCandle);

/// The text of Java's `DailyCandle.toString`: the text of [`Candle`](super::Candle) named `DailyCandle`.
impl std::fmt::Display for DailyCandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DailyCandle{")?;
        self.fmt_candle(f)?;
        f.write_str("}")
    }
}
