//! Candle events.

use dxfeed_graal_sys as sys;

candle_fields! {
    []
    event_type! {
        head {
            /// A candle: the open, high, low and close prices and the volume of a symbol for a period (`Candle`).
            ///
            /// The symbol is the text of a candle symbol, such as `AAPL{=d}`.
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
