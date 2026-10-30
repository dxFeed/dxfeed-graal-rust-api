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

/// The text of Java's `DailyCandle.toString`: the text of [`Candle`](super::Candle) named `DailyCandle`.
impl std::fmt::Display for DailyCandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DailyCandle{")?;
        self.fmt_candle(f)?;
        f.write_str("}")
    }
}
