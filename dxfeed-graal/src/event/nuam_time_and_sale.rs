//! Nuam time and sale events.

use dxfeed_graal_sys as sys;

time_and_sale_fields! {
    [time_and_sale_base]
    event_type! {
        head {
            /// A time and sale of the Nuam exchange, with the identifier of the match (`NuamTimeAndSale`).
            NuamTimeAndSale, NuamTimeAndSaleRef, sys::dxfg_nuam_time_and_sale_t, sys::DXFG_EVENT_NUAM_TIME_AND_SALE,
            [time_and_sale_base.market_event]
        }
        fields {
            /// The identifier of the match between the buyer and the seller.
            match_id: i64 [match_id] = 0,
        }
        derived {}
    }
}

/// The text of Java's `NuamTimeAndSale.toString`: the text of [`TimeAndSale`](super::TimeAndSale) with the match at
/// the end.
impl std::fmt::Display for NuamTimeAndSale {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NuamTimeAndSale{")?;
        self.fmt_time_and_sale(f)?;
        write!(f, ", matchId={}}}", self.match_id)
    }
}
