//! Spread order events.

use dxfeed_graal_sys as sys;

use super::format::Quoted;

order_base_fields! {
    [order_base]
    event_type! {
        head {
            /// An order or a price level of an order book for a multi-leg spread (`SpreadOrder`).
            SpreadOrder, SpreadOrderRef, sys::dxfg_spread_order_t, sys::DXFG_EVENT_SPREAD_ORDER,
            [order_base.market_event]
        }
        fields {
            /// The symbol of the spread; `None` if unknown.
            #[doc(alias = "getSpreadSymbol")]
            spread_symbol: string [spread_symbol] = None,
        }
        derived {}
    }
}

/// The text of Java's `SpreadOrder.toString`, e.g. `SpreadOrder{AAPL, eventTime=0, source=DEFAULT, ...}`.
impl std::fmt::Display for SpreadOrder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SpreadOrder{")?;
        self.fmt_order_base(f)?;
        write!(f, ", spreadSymbol={}}}", Quoted(self.spread_symbol.as_deref()))
    }
}
