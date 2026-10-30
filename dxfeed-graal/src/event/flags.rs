//! Packed fields: bit ranges, the enums kept in them, and the time and sequence packed into one number.

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
