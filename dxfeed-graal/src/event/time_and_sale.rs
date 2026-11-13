//! Time and sale events.

use dxfeed_graal_sys as sys;

time_and_sale_fields! {
    []
    event_type! {
        head {
            /// A trade or another market event with a price: one of the time and sale events of a symbol
            /// (`TimeAndSale`).
            ///
            /// # Examples
            ///
            /// The sales of the last hour, then the new ones:
            ///
            /// ```no_run
            /// # fn main() -> dxfeed_graal::Result<()> {
            /// use std::time::{Duration, SystemTime};
            ///
            /// use dxfeed_graal::DXFeed;
            /// use dxfeed_graal::event::TimeAndSale;
            ///
            /// let hour_ago = SystemTime::now() - Duration::from_secs(3600);
            /// let from_time = hour_ago.duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis() as i64;
            /// let subscription = DXFeed::get_instance()?.create_time_series_subscription::<TimeAndSale>()?;
            /// subscription.add_event_listener(|sales| {
            ///     for sale in sales {
            ///         let (price, size, side) = (sale.price(), sale.size(), sale.aggressor_side());
            ///         println!("{price} x {size} on {:?}, aggressor {side}", sale.exchange_code());
            ///     }
            /// })?;
            /// subscription.set_from_time(from_time)?;
            /// subscription.add_symbols(["AAPL"])?;
            /// # Ok(())
            /// # }
            /// ```
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
