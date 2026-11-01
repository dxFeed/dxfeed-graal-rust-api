//! Greeks events.

use dxfeed_graal_sys as sys;

use super::format::{JavaDouble, Time};

time_series_fields! {
    []
    event_type! {
        head {
            /// The greeks of an option: the sensitivities of its price to the parameters of the market (`Greeks`).
            Greeks, GreeksRef, sys::dxfg_greeks_t, sys::DXFG_EVENT_GREEKS, [market_event]
        }
        fields {
            /// The price of the option; NaN if unknown.
            #[doc(alias = "getPrice")]
            price: f64 [price] = f64::NAN,
            /// The Black-Scholes implied volatility of the option; NaN if unknown.
            #[doc(alias = "getVolatility")]
            volatility: f64 [volatility] = f64::NAN,
            /// The change of the price for a change of the underlying price; NaN if unknown.
            #[doc(alias = "getDelta")]
            delta: f64 [delta] = f64::NAN,
            /// The change of delta for a change of the underlying price; NaN if unknown.
            #[doc(alias = "getGamma")]
            gamma: f64 [gamma] = f64::NAN,
            /// The change of the price for a change of the time to expiration; NaN if unknown.
            #[doc(alias = "getTheta")]
            theta: f64 [theta] = f64::NAN,
            /// The change of the price for a change of the interest rate; NaN if unknown.
            #[doc(alias = "getRho")]
            rho: f64 [rho] = f64::NAN,
            /// The change of the price for a change of the volatility; NaN if unknown.
            #[doc(alias = "getVega")]
            vega: f64 [vega] = f64::NAN,
        }
        derived {}
    }
}

/// The text of Java's `Greeks.toString`, e.g. `Greeks{AAPL, eventTime=0, eventFlags=0x0, time=..., ...}`.
impl std::fmt::Display for Greeks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Greeks{{{}, eventTime={}, eventFlags=0x{:x}, time={}, sequence={}, price={}, volatility={}, delta={}, \
             gamma={}, theta={}, rho={}, vega={}}}",
            self.symbol,
            Time(self.event_time),
            self.event_flags,
            Time(self.time()),
            self.sequence(),
            JavaDouble(self.price),
            JavaDouble(self.volatility),
            JavaDouble(self.delta),
            JavaDouble(self.gamma),
            JavaDouble(self.theta),
            JavaDouble(self.rho),
            JavaDouble(self.vega),
        )
    }
}
