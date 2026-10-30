//! Order base events.

use dxfeed_graal_sys as sys;

order_base_fields! {
    []
    event_type! {
        head {
            /// The fields that all the orders share (`OrderBase`). Feeds deliver its subclasses, such as [`Order`];
            /// publishing an `OrderBase` itself fails with [`Error::Java`](crate::Error::Java), as Java's
            /// `publishEvents` does.
            ///
            /// [`Order`]: super::Order
            OrderBase, OrderBaseRef, sys::dxfg_order_base_t, sys::DXFG_EVENT_ORDER_BASE, [market_event]
        }
        fields {}
        derived {}
    }
}

/// The text of Java's `OrderBase.toString`, e.g. `OrderBase{AAPL, eventTime=0, source=DEFAULT, eventFlags=0x0, ...}`.
impl std::fmt::Display for OrderBase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OrderBase{")?;
        self.fmt_order_base(f)?;
        f.write_str("}")
    }
}
