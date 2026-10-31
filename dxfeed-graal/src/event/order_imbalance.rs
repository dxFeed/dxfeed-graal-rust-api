//! Order imbalance events.

use dxfeed_graal_sys as sys;

use super::flags::{IMBALANCE_AUCTION_TYPE, IMBALANCE_SIDE};
use super::format::{JavaDouble, Time};
use super::{AuctionType, ImbalanceSide, IndexedEventSource};

time_sequence_values! {
    time_sequence
    event_type! {
        head {
            /// The imbalance of the buy and sell orders before an auction (`OrderImbalance`).
            OrderImbalance, OrderImbalanceRef, sys::dxfg_order_imbalance_t, sys::DXFG_EVENT_ORDER_IMBALANCE,
            [market_event]
        }
        fields {
            /// The id of the [`source`](Self::source) of the event, such as `0` for `DEFAULT` or `0x4e5456` for
            /// `NTV`.
            source_id: i32 [source_id] = 0,
            /// The time and the sequence of the event, packed as [`time`](Self::time) and
            /// [`sequence`](Self::sequence) read them.
            time_sequence: i64 [time_sequence] = 0,
            /// The reference price of the auction; NaN if unknown.
            ref_price: f64 [ref_price] = f64::NAN,
            /// The size of the paired orders at the reference price; NaN if unknown.
            paired_size: f64 [paired_size] = f64::NAN,
            /// The size of the imbalance at the reference price; NaN if unknown.
            imbalance_size: f64 [imbalance_size] = f64::NAN,
            /// The price at which the auction would cross with the orders on the book; NaN if unknown.
            near_price: f64 [near_price] = f64::NAN,
            /// The price at which the auction would cross with the auction orders only; NaN if unknown.
            far_price: f64 [far_price] = f64::NAN,
            /// The flags of the event, packed as the SDK keeps them: the auction type in bits 3 to 5, the side of the
            /// imbalance in bits 0 to 2.
            flags: i32 [flags] = 0,
        }
        derived {
            /// The source of the event, such as `NTV`.
            #[doc(alias = "getSource")]
            fn source(source_id) -> IndexedEventSource {
                IndexedEventSource::of(source_id)
            }

            /// The side of the imbalance.
            #[doc(alias = "getImbalanceSide")]
            fn imbalance_side(flags) -> ImbalanceSide {
                ImbalanceSide::from_code(IMBALANCE_SIDE.get32(flags))
            }

            /// The type of the auction.
            #[doc(alias = "getAuctionType")]
            fn auction_type(flags) -> AuctionType {
                AuctionType::from_code(IMBALANCE_AUCTION_TYPE.get32(flags))
            }
        }
    }
}

impl OrderImbalance {
    /// Changes the source of the event.
    #[doc(alias = "setSource")]
    pub fn set_source(&mut self, source: IndexedEventSource) {
        self.source_id = source.get_id();
    }

    /// Changes the side of the imbalance.
    #[doc(alias = "setImbalanceSide")]
    pub fn set_imbalance_side(&mut self, side: ImbalanceSide) {
        self.flags = IMBALANCE_SIDE.set32(self.flags, side.get_code());
    }

    /// Changes the type of the auction.
    #[doc(alias = "setAuctionType")]
    pub fn set_auction_type(&mut self, auction_type: AuctionType) {
        self.flags = IMBALANCE_AUCTION_TYPE.set32(self.flags, auction_type.get_code());
    }
}

/// The text of Java's `OrderImbalance.toString`, e.g. `OrderImbalance{AAPL, eventTime=0, source=DEFAULT, ...}`.
impl std::fmt::Display for OrderImbalance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "OrderImbalance{{{}, eventTime={}, source={}, time={}, sequence={}, refPrice={}, pairedSize={}, \
             imbalanceSize={}, nearPrice={}, farPrice={}, imbalanceSide={}, auctionType={}}}",
            self.symbol,
            Time(self.event_time),
            self.source(),
            Time(self.time()),
            self.sequence(),
            JavaDouble(self.ref_price),
            JavaDouble(self.paired_size),
            JavaDouble(self.imbalance_size),
            JavaDouble(self.near_price),
            JavaDouble(self.far_price),
            self.imbalance_side(),
            self.auction_type(),
        )
    }
}
