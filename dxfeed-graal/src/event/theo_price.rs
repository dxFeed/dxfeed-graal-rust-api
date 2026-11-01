//! Theoretical price events.

use dxfeed_graal_sys as sys;

use super::format::{JavaDouble, Time};

time_series_fields! {
    []
    event_type! {
        head {
            /// The theoretical price of an option, from a pricing model (`TheoPrice`).
            TheoPrice, TheoPriceRef, sys::dxfg_theo_price_t, sys::DXFG_EVENT_THEO_PRICE, [market_event]
        }
        fields {
            /// The theoretical price; NaN if unknown.
            #[doc(alias = "getPrice")]
            price: f64 [price] = f64::NAN,
            /// The price of the underlying the theoretical price was computed for; NaN if unknown.
            #[doc(alias = "getUnderlyingPrice")]
            underlying_price: f64 [underlying_price] = f64::NAN,
            /// The change of the price for a change of the underlying price; NaN if unknown.
            #[doc(alias = "getDelta")]
            delta: f64 [delta] = f64::NAN,
            /// The change of delta for a change of the underlying price; NaN if unknown.
            #[doc(alias = "getGamma")]
            gamma: f64 [gamma] = f64::NAN,
            /// The implied simple dividend return of the underlying; NaN if unknown.
            #[doc(alias = "getDividend")]
            dividend: f64 [dividend] = f64::NAN,
            /// The implied simple interest return of the underlying; NaN if unknown.
            #[doc(alias = "getInterest")]
            interest: f64 [interest] = f64::NAN,
        }
        derived {}
    }
}

/// The text of Java's `TheoPrice.toString`, e.g. `TheoPrice{AAPL, eventTime=0, eventFlags=0x0, time=..., ...}`.
impl std::fmt::Display for TheoPrice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "TheoPrice{{{}, eventTime={}, eventFlags=0x{:x}, time={}, sequence={}, price={}, underlyingPrice={}, \
             delta={}, gamma={}, dividend={}, interest={}}}",
            self.symbol,
            Time(self.event_time),
            self.event_flags,
            Time(self.time()),
            self.sequence(),
            JavaDouble(self.price),
            JavaDouble(self.underlying_price),
            JavaDouble(self.delta),
            JavaDouble(self.gamma),
            JavaDouble(self.dividend),
            JavaDouble(self.interest),
        )
    }
}
