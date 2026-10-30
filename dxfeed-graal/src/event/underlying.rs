//! Underlying events.

use dxfeed_graal_sys as sys;

use super::format::{JavaDouble, Time};

time_series_fields! {
    []
    event_type! {
        head {
            /// The implied volatility and the option volumes of an underlying symbol (`Underlying`).
            Underlying, UnderlyingRef, sys::dxfg_underlying_t, sys::DXFG_EVENT_UNDERLYING, [market_event]
        }
        fields {
            /// The 30-day implied volatility; NaN if unknown.
            volatility: f64 [volatility] = f64::NAN,
            /// The front month implied volatility; NaN if unknown.
            front_volatility: f64 [front_volatility] = f64::NAN,
            /// The back month implied volatility; NaN if unknown.
            back_volatility: f64 [back_volatility] = f64::NAN,
            /// The call options traded today; NaN if unknown.
            call_volume: f64 [call_volume] = f64::NAN,
            /// The put options traded today; NaN if unknown.
            put_volume: f64 [put_volume] = f64::NAN,
            /// The ratio of the put volume to the call volume over 30 days; NaN if unknown.
            put_call_ratio: f64 [put_call_ratio] = f64::NAN,
        }
        derived {}
    }
}

/// The text of Java's `Underlying.toString`, e.g. `Underlying{AAPL, eventTime=0, eventFlags=0x0, time=..., ...}`.
impl std::fmt::Display for Underlying {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Underlying{{{}, eventTime={}, eventFlags=0x{:x}, time={}, sequence={}, volatility={}, \
             frontVolatility={}, backVolatility={}, callVolume={}, putVolume={}, putCallRatio={}}}",
            self.symbol,
            Time(self.event_time),
            self.event_flags,
            Time(self.time()),
            self.sequence(),
            JavaDouble(self.volatility),
            JavaDouble(self.front_volatility),
            JavaDouble(self.back_volatility),
            JavaDouble(self.call_volume),
            JavaDouble(self.put_volume),
            JavaDouble(self.put_call_ratio),
        )
    }
}
