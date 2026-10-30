//! Packed fields: bit ranges, the enums kept in them, and the time and sequence packed into one number.

use std::fmt::{self, LowerHex};
use std::ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign};

use crate::error::{Error, Result};

/// A range of bits of a packed field, read and written as Java's `Util.getBits` and `Util.setBits` do.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Bits {
    mask: i64,
    shift: u32,
}

impl Bits {
    /// The `width` bits starting at bit `shift`.
    pub(crate) const fn new(width: u32, shift: u32) -> Self {
        Bits {
            mask: (1 << width) - 1,
            shift,
        }
    }

    /// The bits of `value`.
    pub(crate) fn get(self, value: i64) -> i32 {
        ((value >> self.shift) & self.mask) as i32
    }

    /// `value` with the bits replaced by the low bits of `bits`; the other bits are kept.
    pub(crate) fn set(self, value: i64, bits: i32) -> i64 {
        (value & !(self.mask << self.shift)) | ((i64::from(bits) & self.mask) << self.shift)
    }

    /// [`Bits::set`] for a 32-bit field; the bits must lie within the low 32.
    pub(crate) fn set32(self, value: i32, bits: i32) -> i32 {
        self.set(value.into(), bits) as i32
    }
}

/// The tick direction in the flags of a trade.
pub(crate) const TRADE_DIRECTION: Bits = Bits::new(3, 1);
/// The extended trading hours bit in the flags of a trade.
pub(crate) const TRADE_ETH: Bits = Bits::new(1, 0);

/// The short sale restriction in the flags of a profile.
pub(crate) const PROFILE_SSR: Bits = Bits::new(2, 2);
/// The trading status in the flags of a profile.
pub(crate) const PROFILE_STATUS: Bits = Bits::new(2, 0);
/// The type of the day close price in the flags of a summary.
pub(crate) const SUMMARY_DAY_CLOSE_TYPE: Bits = Bits::new(2, 2);
/// The type of the previous day close price in the flags of a summary.
pub(crate) const SUMMARY_PREV_DAY_CLOSE_TYPE: Bits = Bits::new(2, 0);

/// The trade-through-exempt character in the flags of a sale.
pub(crate) const SALE_TTE: Bits = Bits::new(8, 8);
/// The aggressor side in the flags of a sale.
pub(crate) const SALE_SIDE: Bits = Bits::new(2, 5);
/// The spread leg bit in the flags of a sale.
pub(crate) const SALE_SPREAD_LEG: Bits = Bits::new(1, 4);
/// The extended trading hours bit in the flags of a sale.
pub(crate) const SALE_ETH: Bits = Bits::new(1, 3);
/// The valid tick bit in the flags of a sale.
pub(crate) const SALE_VALID_TICK: Bits = Bits::new(1, 2);
/// The type of a sale in its flags.
pub(crate) const SALE_TYPE: Bits = Bits::new(2, 0);

/// The action in the flags of an order.
pub(crate) const ORDER_ACTION: Bits = Bits::new(4, 11);
/// The exchange code, a 7-bit ASCII character, in the flags of an order.
pub(crate) const ORDER_EXCHANGE: Bits = Bits::new(7, 4);
/// The side in the flags of an order.
pub(crate) const ORDER_SIDE: Bits = Bits::new(2, 2);
/// The scope in the flags of an order.
pub(crate) const ORDER_SCOPE: Bits = Bits::new(2, 0);
/// The iceberg type in the iceberg flags of an analytic order.
pub(crate) const ICEBERG_TYPE: Bits = Bits::new(2, 0);
/// The NMS conditional bit in the flags of an OTC Markets order.
pub(crate) const OTC_NMS_CONDITIONAL: Bits = Bits::new(1, 6);
/// The auto execution bit in the flags of an OTC Markets order.
pub(crate) const OTC_AUTO_EXECUTION: Bits = Bits::new(1, 5);
/// The saturated bit in the flags of an OTC Markets order.
pub(crate) const OTC_SATURATED: Bits = Bits::new(1, 4);
/// The price type in the flags of an OTC Markets order.
pub(crate) const OTC_PRICE_TYPE: Bits = Bits::new(2, 2);
/// The unsolicited bit in the flags of an OTC Markets order.
pub(crate) const OTC_UNSOLICITED: Bits = Bits::new(1, 1);
/// The open bit in the flags of an OTC Markets order.
pub(crate) const OTC_OPEN: Bits = Bits::new(1, 0);
/// The time in force in the flags of a Nuam order.
pub(crate) const NUAM_TIME_IN_FORCE: Bits = Bits::new(4, 4);
/// The order type in the flags of a Nuam order.
pub(crate) const NUAM_ORDER_TYPE: Bits = Bits::new(4, 0);
/// The side of the imbalance in the flags of an order imbalance.
pub(crate) const IMBALANCE_SIDE: Bits = Bits::new(3, 0);
/// The auction type in the flags of an order imbalance.
pub(crate) const IMBALANCE_AUCTION_TYPE: Bits = Bits::new(3, 3);
/// The exchange code, a Java `char`, in the index of a market maker.
pub(crate) const MARKET_MAKER_EXCHANGE: Bits = Bits::new(16, 32);
/// The packed market maker identifier in the index of a market maker.
pub(crate) const MARKET_MAKER_ID: Bits = Bits::new(32, 0);

/// The names of the special order sources, ids 1 to 9, as Java's `OrderSource` names them.
const SPECIAL_SOURCES: [&str; 9] = [
    "COMPOSITE_BID",
    "COMPOSITE_ASK",
    "REGIONAL_BID",
    "REGIONAL_ASK",
    "AGGREGATE_BID",
    "AGGREGATE_ASK",
    "COMPOSITE",
    "REGIONAL",
    "AGGREGATE",
];

/// Whether `id` is a special order source (`COMPOSITE_BID` to `AGGREGATE`), kept in bits 48 to 63 of an index.
fn is_special_source(id: i64) -> bool {
    (1..=9).contains(&id)
}

/// The id of the order source in the index of an order, as Java's `OrderBase.getSource` reads it: a special source
/// in bits 48 to 63, any other in bits 32 to 63.
pub(crate) fn source_of(index: i64) -> i32 {
    let special = index >> 48;
    if is_special_source(special) {
        special as i32
    } else {
        (index >> 32) as i32
    }
}

/// `index` with another order source, as Java's `OrderBase.setSource` writes it: the low bits of the current format are
/// kept.
pub(crate) fn with_source(index: i64, id: i32) -> i64 {
    let keep = if is_special_source(index >> 48) {
        !(-1 << 48)
    } else {
        !(-1 << 32)
    };
    let shift = if is_special_source(id.into()) { 48 } else { 32 };
    (i64::from(id) << shift) | (index & keep)
}

/// The name of the order source `id`, as Java's `OrderSource.valueOf(id)` gives it: `DEFAULT`, a special source, or one
/// to four letters and digits packed big-endian; `None` if Java refuses the id.
pub(crate) fn source_name(id: i32) -> Option<String> {
    match id {
        0 => return Some("DEFAULT".to_owned()),
        1..=9 => return Some(SPECIAL_SOURCES[id as usize - 1].to_owned()),
        _ => {}
    }
    let bytes = id.to_be_bytes();
    let name = &bytes[bytes.iter().position(|&b| b != 0)?..];
    name.iter()
        .all(u8::is_ascii_alphanumeric)
        .then(|| String::from_utf8_lossy(name).into_owned())
}

/// The market maker identifier packed in `bits`, as Java's `ShortString.decode` reads it: big-endian characters
/// without the zero bytes; `None` for `0`.
pub(crate) fn decode_short_string(bits: i32) -> Option<String> {
    let text: String = bits
        .to_be_bytes()
        .iter()
        .filter(|&&b| b != 0)
        .map(|&b| char::from(b))
        .collect();
    (!text.is_empty()).then_some(text)
}

/// The market maker identifier packed as Java's `ShortString.encode` packs it.
pub(crate) fn encode_short_string(text: &str) -> Result<i32> {
    if text.len() > 4 || !text.bytes().all(|b| b.is_ascii() && b != 0) {
        return Err(Error::InvalidArgument(format!(
            "market maker {text:?} is not up to 4 ASCII characters"
        )));
    }
    Ok(text.bytes().fold(0, |bits, b| (bits << 8) | i32::from(b)))
}

/// The largest sequence that fits the packed time and sequence fields.
pub(crate) const MAX_SEQUENCE: i32 = (1 << 22) - 1;
/// The sequence in the low bits of a packed time and sequence.
const SEQUENCE: Bits = Bits::new(22, 0);
/// The milliseconds of a packed time and sequence.
const MILLIS: Bits = Bits::new(10, 22);

/// The time in milliseconds of a packed time and sequence: seconds in bits 32 to 63, milliseconds in bits 22 to 31.
pub(crate) fn time_of(time_sequence: i64) -> i64 {
    (time_sequence >> 32) * 1000 + i64::from(MILLIS.get(time_sequence))
}

/// The sequence of a packed time and sequence (or of a packed milliseconds and sequence).
pub(crate) fn sequence_of(time_sequence: i64) -> i32 {
    SEQUENCE.get(time_sequence)
}

/// `time_sequence` with another time, as Java's `setTime` computes it: the seconds are clamped to 32 bits.
pub(crate) fn with_time(time_sequence: i64, time: i64) -> i64 {
    let seconds = time.div_euclid(1000).clamp(i32::MIN.into(), i32::MAX.into());
    let millis = time.rem_euclid(1000);
    (seconds << 32) | (millis << 22) | i64::from(sequence_of(time_sequence))
}

/// `time_sequence` with another sequence; Java's `setSequence` refuses a sequence outside 0 to [`MAX_SEQUENCE`].
pub(crate) fn with_sequence(time_sequence: i64, sequence: i32) -> Result<i64> {
    if !(0..=MAX_SEQUENCE).contains(&sequence) {
        return Err(Error::InvalidArgument(format!(
            "sequence {sequence} is outside 0 to {MAX_SEQUENCE}"
        )));
    }
    Ok(SEQUENCE.set(time_sequence, sequence))
}

/// Nanoseconds since the Unix epoch from milliseconds and the nanoseconds part, wrapping as Java's `long` does.
pub(crate) fn nanos(millis: i64, nano_part: i32) -> i64 {
    millis.wrapping_mul(1_000_000).wrapping_add(nano_part.into())
}

/// Milliseconds and the nanoseconds part (0 to 999 999) of nanoseconds since the Unix epoch.
pub(crate) fn split_nanos(nanos: i64) -> (i64, i32) {
    (nanos.div_euclid(1_000_000), nanos.rem_euclid(1_000_000) as i32)
}

/// Defines a public enum of the codes a packed field keeps, with the Java names as `Display`. The first value must
/// have code 0: [`from_code`] gives it for a code without a value, as the C++ API does.
macro_rules! flag_enum {
    (
        $(#[$doc:meta])*
        $name:ident {
            $(#[$fdoc:meta])* $first:ident = 0 $fjava:literal,
            $( $(#[$vdoc:meta])* $variant:ident = $code:literal $java:literal, )*
        }
    ) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
        #[non_exhaustive]
        pub enum $name {
            $(#[$fdoc])*
            #[default]
            $first,
            $( $(#[$vdoc])* $variant, )*
        }

        impl $name {
            /// The code of the value, as Java's `getCode` gives it.
            #[doc(alias = "getCode")]
            pub const fn code(self) -> i32 {
                match self {
                    $name::$first => 0,
                    $( $name::$variant => $code, )*
                }
            }

            #[doc = concat!(
                "The value with `code`, as Java's `", stringify!($name), ".valueOf` gives it; a code without a value ",
                "gives [`", stringify!($name), "::", stringify!($first), "`]."
            )]
            pub const fn from_code(code: i32) -> Self {
                match code {
                    $( $code => $name::$variant, )*
                    _ => $name::$first,
                }
            }
        }

        /// The Java name of the value.
        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.write_str(match self {
                    $name::$first => $fjava,
                    $( $name::$variant => $java, )*
                })
            }
        }
    };
}

flag_enum! {
    /// The direction of the price movement from the previous trade (`Direction`).
    Direction {
        /// The direction is unknown.
        Undefined = 0 "UNDEFINED",
        /// The price went down.
        Down = 1 "DOWN",
        /// The price did not change, and the last change was down.
        ZeroDown = 2 "ZERO_DOWN",
        /// The price did not change, and there was no change before.
        Zero = 3 "ZERO",
        /// The price did not change, and the last change was up.
        ZeroUp = 4 "ZERO_UP",
        /// The price went up.
        Up = 5 "UP",
    }
}

flag_enum! {
    /// The trading status of a symbol (`TradingStatus`).
    TradingStatus {
        /// The status is unknown.
        Undefined = 0 "UNDEFINED",
        /// Trading is halted.
        Halted = 1 "HALTED",
        /// Trading is active.
        Active = 2 "ACTIVE",
    }
}

flag_enum! {
    /// The short sale restriction of a symbol (`ShortSaleRestriction`).
    ShortSaleRestriction {
        /// The restriction is unknown.
        Undefined = 0 "UNDEFINED",
        /// Short sales are restricted.
        Active = 1 "ACTIVE",
        /// Short sales are not restricted.
        Inactive = 2 "INACTIVE",
    }
}

flag_enum! {
    /// The side of an order or of the aggressor of a trade (`Side`).
    Side {
        /// The side is unknown.
        Undefined = 0 "UNDEFINED",
        /// A buy: a bid order or a trade initiated by a buyer.
        Buy = 1 "BUY",
        /// A sell: an ask order or a trade initiated by a seller.
        Sell = 2 "SELL",
    }
}

flag_enum! {
    /// The scope of an order (`Scope`).
    Scope {
        /// The best bid and offer of all the exchanges.
        Composite = 0 "COMPOSITE",
        /// The best bid and offer of one exchange.
        Regional = 1 "REGIONAL",
        /// A price level of the order book.
        Aggregate = 2 "AGGREGATE",
        /// An individual order.
        Order = 3 "ORDER",
    }
}

flag_enum! {
    /// The action of an order in the order book (`OrderAction`).
    OrderAction {
        /// The action is unknown.
        Undefined = 0 "UNDEFINED",
        /// A new order.
        New = 1 "NEW",
        /// An order replaces an earlier one.
        Replace = 2 "REPLACE",
        /// An order changes its price or size.
        Modify = 3 "MODIFY",
        /// An order is removed.
        Delete = 4 "DELETE",
        /// An order is partially executed.
        Partial = 5 "PARTIAL",
        /// An order is fully executed.
        Execute = 6 "EXECUTE",
        /// A trade not tied to an order of the book.
        Trade = 7 "TRADE",
        /// A trade is cancelled.
        Bust = 8 "BUST",
    }
}

flag_enum! {
    /// The type of an iceberg order (`IcebergType`).
    IcebergType {
        /// The type is unknown.
        Undefined = 0 "UNDEFINED",
        /// An iceberg of the exchange.
        Native = 1 "NATIVE",
        /// An iceberg that the data provider reconstructs.
        Synthetic = 2 "SYNTHETIC",
    }
}

flag_enum! {
    /// The price type of an OTC Markets order (`OtcMarketsPriceType`).
    OtcMarketsPriceType {
        /// An unpriced quote.
        Unpriced = 0 "UNPRICED",
        /// An actual priced quote.
        Actual = 1 "ACTUAL",
        /// An offer wanted or a bid wanted.
        Wanted = 2 "WANTED",
    }
}

flag_enum! {
    /// The type of a Nuam order (`NuamOrderType`).
    NuamOrderType {
        /// The type is unknown.
        Undefined = 0 "UNDEFINED",
        /// A limit order.
        Limit = 1 "LIMIT",
        /// A market order.
        Market = 2 "MARKET",
        /// A market order that becomes a limit order at the price of its execution.
        MarketToLimit = 3 "MARKET_TO_LIMIT",
        /// An order at the best price of the book.
        BestOrder = 4 "BEST_ORDER",
        /// An imbalance order.
        Imbalance = 5 "IMBALANCE",
    }
}

flag_enum! {
    /// The time in force of a Nuam order (`NuamTimeInForceType`).
    NuamTimeInForceType {
        /// The time in force is unknown.
        Undefined = 0 "UNDEFINED",
        /// The order is valid until the end of the day.
        RestOfDay = 1 "REST_OF_DAY",
        /// The order is valid until it is cancelled.
        Gtc = 2 "GTC",
        /// The order is executed at once, and the rest is cancelled.
        ImmediateOrCancel = 3 "IMMEDIATE_OR_CANCEL",
        /// The order is executed at once in full or cancelled.
        FillOrKill = 4 "FILL_OR_KILL",
        /// The order is valid until the end of the session.
        GoodTilSession = 5 "GOOD_TIL_SESSION",
        /// The order is valid for a number of days.
        NumberOfDays = 6 "NUMBER_OF_DAYS",
    }
}

flag_enum! {
    /// The side of an order imbalance (`ImbalanceSide`).
    ImbalanceSide {
        /// The side is unknown.
        Undefined = 0 "UNDEFINED",
        /// More buy orders than sell orders.
        Buy = 1 "BUY",
        /// More sell orders than buy orders.
        Sell = 2 "SELL",
        /// No imbalance.
        NoImbalance = 3 "NO_IMBALANCE",
        /// Too few orders to compute the imbalance.
        InsufficientOrders = 4 "INSUFFICIENT_ORDERS",
        /// The imbalance is paused.
        Paused = 5 "PAUSED",
    }
}

flag_enum! {
    /// The type of an auction (`AuctionType`).
    AuctionType {
        /// The type is unknown.
        Undefined = 0 "UNDEFINED",
        /// The opening auction.
        Opening = 1 "OPENING",
        /// The closing auction.
        Closing = 2 "CLOSING",
        /// An auction that reopens trading after a halt.
        Reopening = 3 "REOPENING",
        /// Another auction.
        Other = 4 "OTHER",
    }
}

flag_enum! {
    /// The type of a time and sale event (`TimeAndSaleType`).
    TimeAndSaleType {
        /// A new event.
        New = 0 "NEW",
        /// A correction of an earlier event.
        Correction = 1 "CORRECTION",
        /// A cancellation of an earlier event.
        Cancel = 2 "CANCEL",
    }
}

flag_enum! {
    /// The type of a price (`PriceType`).
    PriceType {
        /// A regular price.
        Regular = 0 "REGULAR",
        /// An indicative price, for example before the opening.
        Indicative = 1 "INDICATIVE",
        /// A preliminary price, for example before the final close.
        Preliminary = 2 "PRELIMINARY",
        /// The final price.
        Final = 3 "FINAL",
    }
}

/// The flags of an indexed event that mark transactions and snapshots, as the constants of Java's `IndexedEvent`.
///
/// It keeps every bit it is created from, including the bits without a constant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct EventFlags(i32);

impl EventFlags {
    /// The event is a part of a transaction that is not complete yet (`TX_PENDING`).
    pub const TX_PENDING: Self = EventFlags(0x01);
    /// The event removes the event with the same index (`REMOVE_EVENT`).
    pub const REMOVE_EVENT: Self = EventFlags(0x02);
    /// The event starts a snapshot (`SNAPSHOT_BEGIN`).
    pub const SNAPSHOT_BEGIN: Self = EventFlags(0x04);
    /// The event ends a snapshot (`SNAPSHOT_END`).
    pub const SNAPSHOT_END: Self = EventFlags(0x08);
    /// The event ends a snapshot that was cut short (`SNAPSHOT_SNIP`).
    pub const SNAPSHOT_SNIP: Self = EventFlags(0x10);
    /// The event is a part of a snapshot (`SNAPSHOT_MODE`).
    pub const SNAPSHOT_MODE: Self = EventFlags(0x40);

    /// No flags.
    pub const fn empty() -> Self {
        EventFlags(0)
    }

    /// The flags of the bits of `bits`, as Java's `setEventFlags` takes them.
    #[doc(alias = "setEventFlags")]
    pub const fn from_bits(bits: i32) -> Self {
        EventFlags(bits)
    }

    /// The bits of the flags, as Java's `getEventFlags` gives them.
    #[doc(alias = "getEventFlags")]
    pub const fn bits(self) -> i32 {
        self.0
    }

    /// Whether every flag of `other` is set.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl BitOr for EventFlags {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        EventFlags(self.0 | other.0)
    }
}

impl BitOrAssign for EventFlags {
    fn bitor_assign(&mut self, other: Self) {
        self.0 |= other.0;
    }
}

impl BitAnd for EventFlags {
    type Output = Self;

    fn bitand(self, other: Self) -> Self {
        EventFlags(self.0 & other.0)
    }
}

impl BitAndAssign for EventFlags {
    fn bitand_assign(&mut self, other: Self) {
        self.0 &= other.0;
    }
}

/// The bits in hex, as Java's `Integer.toHexString` prints them.
impl LowerHex for EventFlags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        LowerHex::fmt(&self.0, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits_change_only_their_range() {
        let bits = Bits::new(3, 1);
        assert_eq!(bits.get(0b1110), 7);
        assert_eq!(bits.set(-1, 0), !0b1110);
        assert_eq!(bits.set(0, -1), 0b1110);
        assert_eq!(bits.set32(i32::MIN | 1, 5), i32::MIN | 1 | (5 << 1));
        let top = Bits::new(16, 48);
        assert_eq!(top.get(i64::MIN), i32::from(i16::MIN as u16));
        assert_eq!(top.set(-1, 0), 0x0000_FFFF_FFFF_FFFF);
    }

    #[test]
    fn directions_have_java_codes_and_names() {
        let names: Vec<String> = (0..8).map(|code| Direction::from_code(code).to_string()).collect();
        assert_eq!(
            names,
            [
                "UNDEFINED",
                "DOWN",
                "ZERO_DOWN",
                "ZERO",
                "ZERO_UP",
                "UP",
                "UNDEFINED",
                "UNDEFINED"
            ]
        );
        for code in 0..6 {
            assert_eq!(Direction::from_code(code).code(), code);
        }
        assert_eq!(Direction::from_code(-1), Direction::Undefined);
        assert_eq!(Direction::default(), Direction::Undefined);
    }

    #[test]
    fn enums_have_java_codes_and_names() {
        let names = |n: i32, name: fn(i32) -> String| (0..n).map(name).collect::<Vec<_>>().join(" ");
        assert_eq!(
            names(4, |c| TradingStatus::from_code(c).to_string()),
            "UNDEFINED HALTED ACTIVE UNDEFINED"
        );
        assert_eq!(
            names(4, |c| ShortSaleRestriction::from_code(c).to_string()),
            "UNDEFINED ACTIVE INACTIVE UNDEFINED"
        );
        assert_eq!(
            names(5, |c| PriceType::from_code(c).to_string()),
            "REGULAR INDICATIVE PRELIMINARY FINAL REGULAR"
        );
        for code in 0..3 {
            assert_eq!(TradingStatus::from_code(code).code(), code);
            assert_eq!(ShortSaleRestriction::from_code(code).code(), code);
        }
        assert_eq!(
            names(4, |c| Side::from_code(c).to_string()),
            "UNDEFINED BUY SELL UNDEFINED"
        );
        assert_eq!(
            names(4, |c| TimeAndSaleType::from_code(c).to_string()),
            "NEW CORRECTION CANCEL NEW"
        );
        for code in 0..3 {
            assert_eq!(Side::from_code(code).code(), code);
            assert_eq!(TimeAndSaleType::from_code(code).code(), code);
        }
        for code in 0..4 {
            assert_eq!(PriceType::from_code(code).code(), code);
        }
        assert_eq!(
            names(10, |c| OrderAction::from_code(c).to_string()),
            "UNDEFINED NEW REPLACE MODIFY DELETE PARTIAL EXECUTE TRADE BUST UNDEFINED"
        );
        assert_eq!(
            names(5, |c| Scope::from_code(c).to_string()),
            "COMPOSITE REGIONAL AGGREGATE ORDER COMPOSITE"
        );
        assert_eq!(
            names(4, |c| IcebergType::from_code(c).to_string()),
            "UNDEFINED NATIVE SYNTHETIC UNDEFINED"
        );
        assert_eq!(
            names(4, |c| OtcMarketsPriceType::from_code(c).to_string()),
            "UNPRICED ACTUAL WANTED UNPRICED"
        );
        assert_eq!(
            names(7, |c| NuamOrderType::from_code(c).to_string()),
            "UNDEFINED LIMIT MARKET MARKET_TO_LIMIT BEST_ORDER IMBALANCE UNDEFINED"
        );
        assert_eq!(
            names(8, |c| NuamTimeInForceType::from_code(c).to_string()),
            "UNDEFINED REST_OF_DAY GTC IMMEDIATE_OR_CANCEL FILL_OR_KILL GOOD_TIL_SESSION NUMBER_OF_DAYS UNDEFINED"
        );
        assert_eq!(
            names(7, |c| ImbalanceSide::from_code(c).to_string()),
            "UNDEFINED BUY SELL NO_IMBALANCE INSUFFICIENT_ORDERS PAUSED UNDEFINED"
        );
        assert_eq!(
            names(6, |c| AuctionType::from_code(c).to_string()),
            "UNDEFINED OPENING CLOSING REOPENING OTHER UNDEFINED"
        );
        for code in 0..9 {
            assert_eq!(OrderAction::from_code(code).code(), code);
        }
        for code in 0..6 {
            assert_eq!(NuamTimeInForceType::from_code(code).code(), code);
            assert_eq!(ImbalanceSide::from_code(code).code(), code);
        }
        assert_eq!(Scope::from_code(3).code(), 3);
        assert_eq!(AuctionType::from_code(4).code(), 4);
        assert_eq!(NuamOrderType::from_code(5).code(), 5);
        assert_eq!(Scope::default(), Scope::Composite);
        assert_eq!(PriceType::default(), PriceType::Regular);
    }

    #[test]
    fn event_flags_keep_all_bits() {
        let flags = EventFlags::SNAPSHOT_BEGIN | EventFlags::TX_PENDING;
        assert_eq!(flags.bits(), 5);
        assert!(flags.contains(EventFlags::TX_PENDING));
        assert!(!flags.contains(EventFlags::SNAPSHOT_END | EventFlags::TX_PENDING));
        assert_eq!(flags & EventFlags::SNAPSHOT_BEGIN, EventFlags::SNAPSHOT_BEGIN);
        assert_eq!(EventFlags::from_bits(-1).bits(), -1);
        assert_eq!(
            format!("{:x} {:x}", EventFlags::from_bits(0x7f), EventFlags::from_bits(-1)),
            "7f ffffffff"
        );
        assert_eq!(EventFlags::default(), EventFlags::empty());
        let mut flags = EventFlags::SNAPSHOT_MODE;
        flags |= EventFlags::REMOVE_EVENT;
        flags &= EventFlags::REMOVE_EVENT | EventFlags::SNAPSHOT_SNIP;
        assert_eq!(flags, EventFlags::REMOVE_EVENT);
    }

    #[test]
    fn order_sources_decode_as_in_java() {
        let named = |id| source_name(id).unwrap_or_else(|| "THROWS".to_owned());
        let ids = [
            0,
            1,
            2,
            3,
            4,
            5,
            6,
            7,
            8,
            9,
            10,
            100,
            12345,
            65535,
            65536,
            0x4e5456,
            0x6e7476,
            0x41,
            0x4142,
            0x414243,
            0x4142_4344,
            0x5a39,
            0x6162_6364,
            -1,
            i32::MAX,
        ];
        assert_eq!(
            ids.map(named).join(" "),
            "DEFAULT COMPOSITE_BID COMPOSITE_ASK REGIONAL_BID REGIONAL_ASK AGGREGATE_BID AGGREGATE_ASK COMPOSITE \
             REGIONAL AGGREGATE THROWS d 09 THROWS THROWS NTV ntv A AB ABC ABCD Z9 abcd THROWS THROWS"
        );
        // The index formats of Java's reference strings.
        assert_eq!(with_source(17, 0x4e5456), 0x4e_5456_0000_0011);
        assert_eq!(with_source(17, 3), 0x3_0000_0000_0011);
        assert_eq!(with_source(0x3_0000_0000_0011, 0x41), 0x41_0000_0011);
        assert_eq!(with_source(0x4e_5456_0000_0011, 0), 17);
        assert_eq!(source_of(0x4e_5456_0000_0011), 0x4e5456);
        assert_eq!(source_of(0x9_0000_0000_0000), 9);
        assert_eq!(source_of(0x4142_4344_0000_0000), 0x4142_4344);
        assert_eq!(source_of(17), 0);
    }

    #[test]
    fn market_makers_pack_as_in_java() {
        assert_eq!(encode_short_string("MM1").unwrap(), 0x4d4d31);
        assert_eq!(decode_short_string(0x4d4d31).as_deref(), Some("MM1"));
        assert_eq!(decode_short_string(0x4d00_4d31).as_deref(), Some("MM1"));
        assert_eq!(decode_short_string(0), None);
        assert_eq!(encode_short_string("").unwrap(), 0);
        assert_eq!(
            decode_short_string(encode_short_string("ABCD").unwrap()).as_deref(),
            Some("ABCD")
        );
        for bad in ["ABCDE", "\u{e9}", "A\0"] {
            assert!(
                matches!(encode_short_string(bad), Err(Error::InvalidArgument(_))),
                "{bad:?}"
            );
        }
        assert_eq!(MARKET_MAKER_EXCHANGE.get(0x51_004d_4d31), 0x51);
        assert_eq!(MARKET_MAKER_ID.get(0x51_004d_4d31), 0x4d4d31);
    }

    #[test]
    fn time_and_sequence_pack_as_in_java() {
        let packed = with_time(0, 1_700_000_000_789);
        assert_eq!(time_of(packed), 1_700_000_000_789);
        assert_eq!(time_of(with_time(0, -1)), -1);
        assert_eq!(time_of(with_time(0, i64::MAX)), i64::from(i32::MAX) * 1000 + 807);
        let packed = with_sequence(packed, MAX_SEQUENCE).unwrap();
        assert_eq!(sequence_of(packed), MAX_SEQUENCE);
        assert_eq!(time_of(packed), 1_700_000_000_789);
        assert_eq!(sequence_of(with_time(packed, 5)), MAX_SEQUENCE);
        assert!(matches!(with_sequence(0, -1), Err(Error::InvalidArgument(_))));
        assert!(matches!(
            with_sequence(0, MAX_SEQUENCE + 1),
            Err(Error::InvalidArgument(_))
        ));
        assert_eq!(nanos(1, 2), 1_000_002);
        assert_eq!(split_nanos(-1), (-1, 999_999));
        assert_eq!(nanos(i64::MAX, 0), i64::MAX.wrapping_mul(1_000_000));
    }
}
