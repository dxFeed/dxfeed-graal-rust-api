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
