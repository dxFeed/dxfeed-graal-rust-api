//! Option sale events.

use dxfeed_graal_sys as sys;

use super::format::{JavaChar, JavaDouble, Quoted, Time};

sale_fields! {
    [] time_sequence
    event_type! {
        head {
            /// A trade or another market event with a price for an option of an underlying symbol (`OptionSale`).
            OptionSale, OptionSaleRef, sys::dxfg_option_sale_t, sys::DXFG_EVENT_OPTION_SALE, [market_event]
        }
        fields {
            /// The price of the underlying at the time of the event; NaN if unknown.
            #[doc(alias = "getUnderlyingPrice")]
            underlying_price: f64 [underlying_price] = f64::NAN,
            /// The Black-Scholes implied volatility of the option at the time of the event; NaN if unknown.
            #[doc(alias = "getVolatility")]
            volatility: f64 [volatility] = f64::NAN,
            /// The delta of the option at the time of the event; NaN if unknown.
            #[doc(alias = "getDelta")]
            delta: f64 [delta] = f64::NAN,
            /// The symbol of the option; `None` if unknown.
            #[doc(alias = "getOptionSymbol")]
            option_symbol: string [option_symbol] = None,
        }
        derived {}
    }
}

/// The text of Java's `OptionSale.toString`, e.g. `OptionSale{AAPL, eventTime=0, eventFlags=0x0, index=0x11, ...}`.
impl std::fmt::Display for OptionSale {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "OptionSale{{{}, eventTime={}, eventFlags=0x{:x}, index=0x{:x}, time={}, timeNanoPart={}, sequence={}, \
             exchange={}, ",
            self.symbol,
            Time(self.event_time),
            self.event_flags,
            self.index,
            Time(self.time()),
            self.time_nano_part,
            self.sequence(),
            JavaChar(self.exchange_code),
        )?;
        self.fmt_sale(f)?;
        write!(
            f,
            ", underlyingPrice={}, volatility={}, delta={}, optionSymbol={}}}",
            JavaDouble(self.underlying_price),
            JavaDouble(self.volatility),
            JavaDouble(self.delta),
            Quoted(self.option_symbol.as_deref()),
        )
    }
}
