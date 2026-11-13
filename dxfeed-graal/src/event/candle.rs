//! Candle events.

use dxfeed_graal_sys as sys;

candle_fields! {
    []
    event_type! {
        head {
            /// A candle: the open, high, low and close prices and the volume of a symbol for a period (`Candle`).
            ///
            /// The symbol is the text of a candle symbol, such as `AAPL{=d}`.
            ///
            /// # Examples
            ///
            /// The daily candles of the last 20 days, in one request:
            ///
            /// ```no_run
            /// # fn main() -> dxfeed_graal::Result<()> {
            /// use std::time::{Duration, SystemTime};
            ///
            /// use dxfeed_graal::DXFeed;
            /// use dxfeed_graal::event::Candle;
            ///
            /// let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis() as i64;
            /// let from = now - 20 * 24 * 3600 * 1000;
            /// let candles = DXFeed::get_instance()?
            ///     .get_time_series_promise::<Candle>("AAPL{=d}", from, now)?
            ///     .wait(Duration::from_secs(10))?;
            /// for c in &candles {
            ///     println!("{}: O {} H {} L {} C {} V {}", c.symbol, c.open, c.high, c.low, c.close, c.volume);
            /// }
            /// # Ok(())
            /// # }
            /// ```
            ///
            /// A [`DXFeedTimeSeriesSubscription`](crate::DXFeedTimeSeriesSubscription) gets them and their updates,
            /// and [`CandleSymbol`](super::CandleSymbol) builds the symbols.
            Candle, CandleRef, sys::dxfg_candle_t, sys::DXFG_EVENT_CANDLE, []
        }
        fields {}
        derived {}
    }
}

/// The text of Java's `Candle.toString`, e.g. `Candle{AAPL{=d}, eventTime=0, eventFlags=0x0, time=..., ...}`.
impl std::fmt::Display for Candle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Candle{")?;
        self.fmt_candle(f)?;
        f.write_str("}")
    }
}
