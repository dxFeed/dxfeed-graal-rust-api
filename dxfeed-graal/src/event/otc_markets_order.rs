//! OTC Markets order events.

use dxfeed_graal_sys as sys;

use super::OtcMarketsPriceType;
use super::flags::{OTC_AUTO_EXECUTION, OTC_NMS_CONDITIONAL, OTC_OPEN, OTC_PRICE_TYPE, OTC_SATURATED, OTC_UNSOLICITED};
use super::format::Quoted;

order_base_fields! {
    [order_base.order_base]
    event_type! {
        head {
            /// An [`Order`](super::Order) of OTC Markets, with the flags of its quote (`OtcMarketsOrder`).
            OtcMarketsOrder, OtcMarketsOrderRef, sys::dxfg_otc_markets_order_t, sys::DXFG_EVENT_OTC_MARKETS_ORDER,
            [order_base.order_base.market_event]
        }
        fields {
            /// The identifier of the market maker or of another aggregate of the order; `None` if unknown.
            #[doc(alias = "getMarketMaker")]
            market_maker: string [order_base.market_maker] = None,
            /// The quote access payment (QAP): the fee in cents per 100 shares for accessing the quote.
            #[doc(alias = "getQuoteAccessPayment")]
            quote_access_payment: i32 [quote_access_payment] = 0,
            /// The flags of the quote, packed as the SDK keeps them: NMS conditional in bit 6, auto execution in bit
            /// 5, saturated in bit 4, the price type in bits 2 and 3, unsolicited in bit 1, open in bit 0.
            otc_markets_flags: i32 [otc_markets_flags] = 0,
        }
        derived {
            /// Whether the quote is open: it is eligible for execution.
            #[doc(alias = "isOpen")]
            fn is_open(otc_markets_flags) -> bool {
                OTC_OPEN.is_set32(otc_markets_flags)
            }

            /// Whether the quote is unsolicited: a broker enters it for a customer.
            #[doc(alias = "isUnsolicited")]
            fn is_unsolicited(otc_markets_flags) -> bool {
                OTC_UNSOLICITED.is_set32(otc_markets_flags)
            }

            /// The price type of the quote.
            #[doc(alias = "getOtcMarketsPriceType")]
            fn otc_markets_price_type(otc_markets_flags) -> OtcMarketsPriceType {
                OtcMarketsPriceType::from_code(OTC_PRICE_TYPE.get32(otc_markets_flags))
            }

            /// Whether the quote is saturated: a market maker has too many quotes at the price.
            #[doc(alias = "isSaturated")]
            fn is_saturated(otc_markets_flags) -> bool {
                OTC_SATURATED.is_set32(otc_markets_flags)
            }

            /// Whether the quote is eligible for automatic execution.
            #[doc(alias = "isAutoExecution")]
            fn is_auto_execution(otc_markets_flags) -> bool {
                OTC_AUTO_EXECUTION.is_set32(otc_markets_flags)
            }

            /// Whether the quote is NMS conditional: it may be executed only at its full size.
            #[doc(alias = "isNmsConditional")]
            fn is_nms_conditional(otc_markets_flags) -> bool {
                OTC_NMS_CONDITIONAL.is_set32(otc_markets_flags)
            }
        }
    }
}

impl OtcMarketsOrder {
    /// Changes whether the quote is open.
    #[doc(alias = "setOpen")]
    pub fn set_open(&mut self, open: bool) {
        self.otc_markets_flags = OTC_OPEN.set32(self.otc_markets_flags, open.into());
    }

    /// Changes whether the quote is unsolicited.
    #[doc(alias = "setUnsolicited")]
    pub fn set_unsolicited(&mut self, unsolicited: bool) {
        self.otc_markets_flags = OTC_UNSOLICITED.set32(self.otc_markets_flags, unsolicited.into());
    }

    /// Changes the price type of the quote.
    #[doc(alias = "setOtcMarketsPriceType")]
    pub fn set_otc_markets_price_type(&mut self, price_type: OtcMarketsPriceType) {
        self.otc_markets_flags = OTC_PRICE_TYPE.set32(self.otc_markets_flags, price_type.get_code());
    }

    /// Changes whether the quote is saturated.
    #[doc(alias = "setSaturated")]
    pub fn set_saturated(&mut self, saturated: bool) {
        self.otc_markets_flags = OTC_SATURATED.set32(self.otc_markets_flags, saturated.into());
    }

    /// Changes whether the quote is eligible for automatic execution.
    #[doc(alias = "setAutoExecution")]
    pub fn set_auto_execution(&mut self, auto_execution: bool) {
        self.otc_markets_flags = OTC_AUTO_EXECUTION.set32(self.otc_markets_flags, auto_execution.into());
    }

    /// Changes whether the quote is NMS conditional.
    #[doc(alias = "setNmsConditional")]
    pub fn set_nms_conditional(&mut self, nms_conditional: bool) {
        self.otc_markets_flags = OTC_NMS_CONDITIONAL.set32(self.otc_markets_flags, nms_conditional.into());
    }
}

/// The text of Java's `OtcMarketsOrder.toString`, e.g. `OtcMarketsOrder{AAPL, eventTime=0, source=DEFAULT, ...}`.
impl std::fmt::Display for OtcMarketsOrder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OtcMarketsOrder{")?;
        self.fmt_order_base(f)?;
        write!(
            f,
            ", marketMaker={}, QAP={}, open={}, unsolicited={}, priceType={}, saturated={}, autoEx={}, NMS={}}}",
            Quoted(self.market_maker.as_deref()),
            self.quote_access_payment,
            self.is_open(),
            self.is_unsolicited(),
            self.otc_markets_price_type(),
            self.is_saturated(),
            self.is_auto_execution(),
            self.is_nms_conditional(),
        )
    }
}
