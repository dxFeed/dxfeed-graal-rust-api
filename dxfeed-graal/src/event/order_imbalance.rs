//! Order imbalance events.

use dxfeed_graal_sys as sys;

use super::flags::{IMBALANCE_AUCTION_TYPE, IMBALANCE_SIDE};
use super::format::{JavaDouble, SourceName, Time};
use super::{AuctionType, ImbalanceSide};

time_sequence_values! {
    time_sequence
    event_type! {
        head {
            /// The imbalance of the buy and sell orders before an auction (`OrderImbalance`).
            OrderImbalance, OrderImbalanceRef, sys::dxfg_order_imbalance_t, sys::DXFG_EVENT_ORDER_IMBALANCE,
            [market_event]
        }
        fields {
            /// The id of the source of the event: `0` for the default source `DEFAULT`, `1` to `9` for the special
            /// sources (`COMPOSITE_BID` to `AGGREGATE`), or one to four letters and digits packed big-endian, such as
            /// `0x4e5456` for `NTV`. A later version keeps the source itself.
            #[doc(alias = "getSource")]
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
            /// The side of the imbalance.
            #[doc(alias = "getImbalanceSide")]
            fn imbalance_side(flags) -> ImbalanceSide {
                ImbalanceSide::from_code(IMBALANCE_SIDE.get(flags.into()))
            }

            /// The type of the auction.
            #[doc(alias = "getAuctionType")]
            fn auction_type(flags) -> AuctionType {
                AuctionType::from_code(IMBALANCE_AUCTION_TYPE.get(flags.into()))
            }
        }
    }
}

impl OrderImbalance {
    /// Changes the side of the imbalance.
    #[doc(alias = "setImbalanceSide")]
    pub fn set_imbalance_side(&mut self, side: ImbalanceSide) {
        self.flags = IMBALANCE_SIDE.set32(self.flags, side.code());
    }

    /// Changes the type of the auction.
    #[doc(alias = "setAuctionType")]
    pub fn set_auction_type(&mut self, auction_type: AuctionType) {
        self.flags = IMBALANCE_AUCTION_TYPE.set32(self.flags, auction_type.code());
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
            SourceName(self.source_id),
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
