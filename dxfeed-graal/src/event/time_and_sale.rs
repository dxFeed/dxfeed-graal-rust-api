//! Time and sale events.

use dxfeed_graal_sys as sys;

time_and_sale_fields! {
    []
    event_type! {
        head {
            /// A trade or another market event with a price: one of the time and sale events of a symbol
            /// (`TimeAndSale`).
            TimeAndSale, TimeAndSaleRef, sys::dxfg_time_and_sale_t, sys::DXFG_EVENT_TIME_AND_SALE, [market_event]
        }
        fields {}
        derived {}
    }
}

/// The text of Java's `TimeAndSale.toString`, e.g. `TimeAndSale{AAPL, eventTime=0, eventFlags=0x0, time=..., ...}`;
/// the buyer and the seller print only when they are set.
impl std::fmt::Display for TimeAndSale {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TimeAndSale{")?;
        self.fmt_time_and_sale(f)?;
        f.write_str("}")
    }
}
