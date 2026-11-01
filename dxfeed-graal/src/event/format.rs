//! The Java text of event fields, for the `Display` of events: `Double.toString`, `StringUtil.encodeChar`,
//! `WideDecimal`, `TimeFormat` and `DayUtil`.

use std::fmt::{self, Display, Formatter, Write};

use crate::TimeFormat;
use crate::isolate;

/// A `double` as Java's `Double.toString` prints it.
pub(crate) struct JavaDouble(pub(crate) f64);

impl Display for JavaDouble {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let d = self.0;
        if d.is_nan() {
            return f.write_str("NaN");
        }
        if d.is_infinite() {
            return f.write_str(if d > 0.0 { "Infinity" } else { "-Infinity" });
        }
        if d.is_sign_negative() {
            f.write_char('-')?;
        }
        let a = d.abs();
        if a == 0.0 {
            return f.write_str("0.0");
        }
        if (1e-3..1e7).contains(&a) {
            // Rust prints the same shortest digits without an exponent, but no `.0` for whole numbers.
            return if a.fract() == 0.0 {
                write!(f, "{a}.0")
            } else {
                write!(f, "{a}")
            };
        }
        let (digits, exponent) = java_digits(a);
        let (first, rest) = digits.split_at(1);
        let rest = if rest.is_empty() { "0" } else { rest };
        write!(f, "{first}.{rest}E{exponent}")
    }
}

/// The significant digits of a positive finite `a`, without trailing zeros, and the decimal exponent of the first one.
///
/// They are the shortest digits that read back as `a`, as Rust and Java 19+ choose them, with Java's rule that a value
/// whose shortest form has one digit uses the closest two-digit form (`5e-324` is `4.9E-324`).
pub(crate) fn java_digits(a: f64) -> (String, i32) {
    let split = |s: &str| {
        let (mantissa, exponent) = s.split_once('e').expect("exponent notation");
        let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
        (digits, exponent.parse::<i32>().expect("decimal exponent"))
    };
    let (mut digits, mut exponent) = split(&format!("{a:e}"));
    if digits.len() == 1 {
        let two = format!("{a:.1e}");
        if two.parse::<f64>() == Ok(a) {
            (digits, exponent) = split(&two);
        }
    }
    let trimmed = digits.trim_end_matches('0');
    digits.truncate(trimmed.len().max(1));
    (digits, exponent)
}

/// A character as Java's `StringUtil.encodeChar` prints it.
pub(super) struct JavaChar(pub(super) char);

impl Display for JavaChar {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self.0 {
            c @ ' '..='~' => f.write_char(c),
            '\0' => f.write_str("\\0"),
            c => write!(f, "\\u{:04x}", c as u32),
        }
    }
}

/// A time in milliseconds since the Unix epoch, printed by the SDK's `TimeFormat.DEFAULT.withMillis()` in the default
/// time zone, as Java prints the times of events; `0` prints as `0`.
///
/// While the process exits, or if the SDK fails, the time prints as its decimal milliseconds.
pub(super) struct Time(pub(super) i64);

impl Display for Time {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write_time(f, self.0, true)
    }
}

/// A time as [`Time`] prints it, but by `TimeFormat.DEFAULT`, without milliseconds: Java prints the halt times of a
/// profile so.
pub(super) struct TimeSeconds(pub(super) i64);

impl Display for TimeSeconds {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write_time(f, self.0, false)
    }
}

fn write_time(f: &mut Formatter<'_>, millis: i64, with_millis: bool) -> fmt::Result {
    if millis == 0 {
        return f.write_char('0');
    }
    match format_time(millis, with_millis) {
        Some(text) => f.write_str(&text),
        None => write!(f, "{millis}"),
    }
}

fn format_time(millis: i64, with_millis: bool) -> Option<String> {
    if isolate::exiting() {
        return None;
    }
    let format = if with_millis {
        TimeFormat::DEFAULT.with_millis()
    } else {
        TimeFormat::DEFAULT
    };
    format.format(millis).ok()
}

/// A nullable string in single quotes, as Java's `toString` of events prints it: `'text'`, or `'null'`.
pub(super) struct Quoted<'a>(pub(super) Option<&'a str>);

impl Display for Quoted<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "'{}'", self.0.unwrap_or("null"))
    }
}

/// A `double` as Java prints `WideDecimal.toString(WideDecimal.composeWide(d))`: the shares and the free float of a
/// profile. The value is rounded to a decimal with a 56-bit significand; plain notation is used for at most 6 leading
/// or trailing zeros (`1000`, `0.0001`, `1E7`).
pub(super) struct WideDecimal(pub(super) f64);

impl Display for WideDecimal {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(&wide::to_string(wide::compose(self.0)))
    }
}

/// A port of the parts of Java's `com.devexperts.util.WideDecimal` 3.356 that `composeWide(double)` and `toString`
/// use: a 56-bit significand and an 8-bit rank in one `long`, the value being `significand * 10^(BIAS - rank)`; rank
/// 0 is NaN or an infinity by the sign of the significand.
mod wide {
    use std::sync::OnceLock;

    const BIAS: i32 = 128;
    const MAX_SIGNIFICAND: i64 = i64::MAX >> 8;
    const MIN_SIGNIFICAND: i64 = i64::MIN >> 8;
    const MAX_DOUBLE_VALUE: f64 = (1_i64 << 51) as f64;
    const NAN: i64 = 0;
    const POSITIVE_INFINITY: i64 = 0x100;
    const NEGATIVE_INFINITY: i64 = -0x100;
    const EXACT_LONG_POWERS: i32 = 18;
    const MAX_LEADING_ZEROS: i32 = 6;
    const MAX_TRAILING_ZEROS: i32 = 6;
    const SCIENTIFIC_MODULO: i64 = 10_000_000;

    /// `10^e` as Java's `Double.parseDouble("1E" + e)` gives it.
    fn pow10(e: i32) -> f64 {
        // The ranks give exponents from -127 to 127.
        static POWERS: OnceLock<[f64; 256]> = OnceLock::new();
        let powers = POWERS
            .get_or_init(|| std::array::from_fn(|i| format!("1E{}", i as i32 - BIAS).parse().expect("a power of 10")));
        powers[(e + BIAS) as usize]
    }

    fn long_pow10(e: i32) -> i64 {
        10_i64.pow(e as u32)
    }

    /// Java's `MULTIPLIERS[rank]`.
    fn multiplier(rank: i32) -> f64 {
        if rank == 0 { f64::INFINITY } else { pow10(BIAS - rank) }
    }

    /// Java's `DIVISORS[rank]`.
    fn divisor(rank: i32) -> f64 {
        if rank == 0 { 0.0 } else { pow10(rank - BIAS) }
    }

    /// NaN or an infinity by the sign of `significand`.
    fn non_finite(significand: i64) -> i64 {
        match significand.signum() {
            1 => POSITIVE_INFINITY,
            -1 => NEGATIVE_INFINITY,
            _ => NAN,
        }
    }

    /// Java's `composeWide(double)`.
    pub(super) fn compose(value: f64) -> i64 {
        // `as` saturates as Java's `(long)` does, and NaN gives 0 in both.
        if value == value as i64 as f64 {
            compose_long(value as i64)
        } else if value >= 0.0 {
            compose_non_negative(value)
        } else if value < 0.0 {
            neg(compose_non_negative(-value))
        } else {
            NAN
        }
    }

    fn compose_long(value: i64) -> i64 {
        if (MIN_SIGNIFICAND..=MAX_SIGNIFICAND).contains(&value) {
            (value << 8) | i64::from(BIAS)
        } else {
            compose_non_fitting(value, BIAS)
        }
    }

    fn compose_non_negative(value: f64) -> i64 {
        let rank = find_rank(value);
        if rank <= 0 {
            return POSITIVE_INFINITY;
        }
        let significand = (value * divisor(rank) + 0.5) as i64;
        compose_fitting(significand, rank)
    }

    /// Java's `findRank`: a binary search of the largest rank whose limit `value` does not exceed.
    fn find_rank(value: f64) -> i32 {
        let max_double = |rank: i32| MAX_DOUBLE_VALUE * multiplier(rank);
        let mut rank = if value > max_double(128) { 0 } else { 128 };
        for step in [64, 32, 16, 8, 4, 2, 1] {
            if value <= max_double(rank + step) {
                rank += step;
            }
        }
        rank
    }

    /// Java's `composeNonFitting` for the target rank `BIAS`.
    fn compose_non_fitting(mut significand: i64, mut rank: i32) -> i64 {
        let reduction = if (MIN_SIGNIFICAND..=MAX_SIGNIFICAND).contains(&significand) {
            0
        } else if (MIN_SIGNIFICAND * 10 - 5..MAX_SIGNIFICAND * 10 + 5).contains(&significand) {
            1
        } else if (MIN_SIGNIFICAND * 100 - 50..MAX_SIGNIFICAND * 100 + 50).contains(&significand) {
            2
        } else {
            3
        };
        if rank <= reduction {
            return non_finite(significand);
        }
        let reduction = reduction.max(rank - 255);
        if reduction > 0 {
            significand = div10(significand, long_pow10(reduction));
            rank -= reduction;
        }
        compose_fitting(significand, rank)
    }

    /// Java's `composeFitting` for the target rank `BIAS`.
    fn compose_fitting(mut significand: i64, mut rank: i32) -> i64 {
        if significand == 0 {
            return i64::from(BIAS);
        }
        while rank > BIAS && significand % 10 == 0 {
            significand /= 10;
            rank -= 1;
        }
        let fits_one_more_digit = |s: i64| (MIN_SIGNIFICAND / 10..=MAX_SIGNIFICAND / 10).contains(&s);
        while rank < BIAS && fits_one_more_digit(significand) {
            significand *= 10;
            rank += 1;
        }
        // Java keeps one decimal place as two: 0.5 is 50 * 10^-2.
        if rank == BIAS + 1 && fits_one_more_digit(significand) {
            significand *= 10;
            rank += 1;
        }
        (significand << 8) | i64::from(rank)
    }

    /// Java's `div10`: division by a power of 10 that rounds half up, toward negative infinity for negative values.
    fn div10(significand: i64, pow10: i64) -> i64 {
        if significand >= 0 {
            (significand / (pow10 >> 1) + 1) >> 1
        } else {
            ((significand + 1) / (pow10 >> 1)) >> 1
        }
    }

    fn neg(wide: i64) -> i64 {
        (wide ^ -0x100).wrapping_add(0x100)
    }

    /// Java's `WideDecimal.toString(long)`.
    pub(super) fn to_string(wide: i64) -> String {
        let significand = wide >> 8;
        let rank = (wide & 0xFF) as i32;
        if rank == 0 {
            let text = match non_finite(significand) {
                POSITIVE_INFINITY => "Infinity",
                NEGATIVE_INFINITY => "-Infinity",
                _ => "NaN",
            };
            return text.to_owned();
        }
        if significand == 0 {
            return "0".to_owned();
        }
        match to_integer(significand, rank) {
            0 => fractional_or_scientific(significand, rank),
            integer => integer.to_string(),
        }
    }

    /// Java's `toIntegerString`: the value if it is an integer with at most 6 trailing zeros, otherwise 0.
    fn to_integer(significand: i64, rank: i32) -> i64 {
        if rank > BIAS {
            if rank - BIAS <= EXACT_LONG_POWERS {
                let pow10 = long_pow10(rank - BIAS);
                let result = significand / pow10;
                if result * pow10 == significand && result % SCIENTIFIC_MODULO != 0 {
                    return result;
                }
            }
        } else if rank == BIAS {
            return if significand % SCIENTIFIC_MODULO == 0 {
                0
            } else {
                significand
            };
        } else if BIAS - rank <= EXACT_LONG_POWERS.min(MAX_TRAILING_ZEROS) {
            let pow10 = long_pow10(BIAS - rank);
            let result = significand.wrapping_mul(pow10);
            if result / pow10 == significand && result % SCIENTIFIC_MODULO != 0 {
                return result;
            }
        }
        0
    }

    /// Java's `appendFractionalOrScientific`.
    fn fractional_or_scientific(mut significand: i64, mut rank: i32) -> String {
        while significand % 10 == 0 {
            significand /= 10;
            rank -= 1;
        }
        let mut text = significand.to_string();
        let first_digit = i32::from(significand < 0);
        let zeros = |n: i32| "0".repeat(n as usize);
        if rank > BIAS {
            let dot = text.len() as i32 - (rank - BIAS);
            if dot > first_digit {
                text.insert(dot as usize, '.');
                return text;
            }
            if first_digit - dot <= MAX_LEADING_ZEROS {
                text.insert_str(first_digit as usize, &format!("0.{}", zeros(first_digit - dot)));
                return text;
            }
        } else if BIAS - rank <= MAX_TRAILING_ZEROS {
            return text + &zeros(BIAS - rank);
        }
        let digits = text.len() as i32 - first_digit;
        if digits != 1 {
            text.insert((first_digit + 1) as usize, '.');
        }
        format!("{text}E{}", BIAS - rank + digits - 1)
    }
}

/// The `yyyymmdd` number of a day id, the days since 1970-01-01, as Java's `DayUtil.getYearMonthDayByDayId` gives it:
/// proleptic Gregorian, negative for years before 0.
pub(crate) fn year_month_day(day_id: i32) -> i32 {
    // Howard Hinnant's `civil_from_days`.
    let z = i64::from(day_id) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let yyyymmdd = year.abs() * 10_000 + month * 100 + day;
    (if year >= 0 { yyyymmdd } else { -yyyymmdd }) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doubles_print_as_in_java() {
        let cases = [
            (100.0, "100.0"),
            (0.5, "0.5"),
            (1e7, "1.0E7"),
            (12345678.9, "1.23456789E7"),
            (9999999.0, "9999999.0"),
            (0.001, "0.001"),
            (0.0001, "1.0E-4"),
            (1.5e-5, "1.5E-5"),
            (123.456, "123.456"),
            (-2.5, "-2.5"),
            (0.0, "0.0"),
            (-0.0, "-0.0"),
            (f64::NAN, "NaN"),
            (f64::INFINITY, "Infinity"),
            (f64::NEG_INFINITY, "-Infinity"),
            (f64::MIN_POSITIVE, "2.2250738585072014E-308"),
            (5e-324, "4.9E-324"),
            (f64::MAX, "1.7976931348623157E308"),
            (1e21, "1.0E21"),
            (0.1 + 0.2, "0.30000000000000004"),
            (1234567.125, "1234567.125"),
            (0.00123, "0.00123"),
        ];
        for (value, java) in cases {
            assert_eq!(JavaDouble(value).to_string(), java, "{value:?}");
        }
    }

    #[test]
    fn chars_print_as_in_java() {
        assert_eq!(JavaChar('Q').to_string(), "Q");
        assert_eq!(JavaChar(' ').to_string(), " ");
        assert_eq!(JavaChar('\0').to_string(), "\\0");
        assert_eq!(JavaChar('\u{7F}').to_string(), "\\u007f");
        assert_eq!(JavaChar('\u{1F}').to_string(), "\\u001f");
        assert_eq!(JavaChar('\u{e9}').to_string(), "\\u00e9");
    }

    #[test]
    fn day_ids_print_as_in_java() {
        assert_eq!(year_month_day(0), 19700101);
        assert_eq!(year_month_day(19723), 20240101);
        assert_eq!(year_month_day(20000), 20241004);
        assert_eq!(year_month_day(-1), 19691231);
        assert_eq!(year_month_day(11017), 20000301);
        assert_eq!(year_month_day(11016), 20000229);
    }

    #[test]
    fn zero_time_needs_no_sdk() {
        assert_eq!(Time(0).to_string(), "0");
        assert_eq!(TimeSeconds(0).to_string(), "0");
    }

    #[test]
    fn strings_print_quoted_as_in_java() {
        assert_eq!(Quoted(Some("desc")).to_string(), "'desc'");
        assert_eq!(Quoted(Some("")).to_string(), "''");
        assert_eq!(Quoted(None).to_string(), "'null'");
    }

    /// The text and the bits Java 3.356 gives for `WideDecimal.toString(WideDecimal.composeWide(d))`.
    #[test]
    fn wide_decimals_print_as_in_java() {
        let cases = [
            (1000.0, "1000"),
            (0.5, "0.5"),
            (1.0e-4, "0.0001"),
            (1.23456789e10, "12345678900"),
            (f64::NAN, "NaN"),
            (f64::INFINITY, "Infinity"),
            (f64::NEG_INFINITY, "-Infinity"),
            (0.0, "0"),
            (-0.0, "0"),
            (1.0e7, "1E7"),
            (1.23456789e7, "12345678.9"),
            (1.0e-5, "0.00001"),
            (1.2345678901234568e17, "123456789012345680"),
            (5e-324, "0"),
            (f64::MAX, "Infinity"),
            (0.1, "0.1"),
            (100.0, "100"),
            (1.0e15, "1E15"),
            (500.5, "500.5"),
        ];
        for (value, java) in cases {
            assert_eq!(WideDecimal(value).to_string(), java, "{value:?}");
        }
        let bits = [
            (1000.0, 0x3e880),
            (0.5, 0x3282),
            (1.0e-4, 0x184),
            (1.23456789e10, 0x2dfdc1c3480),
            (f64::INFINITY, 0x100),
            (f64::NEG_INFINITY, -0x100),
            (-0.0, 0x80),
            (1.0e7, 0x98968080),
            (1.23456789e7, 0x499602d282),
            (1.2345678901234568e17, 0x2bdc545d6b4b887f),
            (1.0e15, 0x38d7ea4c6800080),
        ];
        for (value, wide) in bits {
            assert_eq!(wide::compose(value), wide, "{value:?}");
        }
        // Negative, tiny and huge values neither panic nor overflow.
        for value in [
            -2.5,
            -1.0e-9,
            1.0e-9,
            -1.0e300,
            1.5e20,
            i64::MAX as f64,
            i64::MIN as f64,
            f64::MIN,
        ] {
            let _ = WideDecimal(value).to_string();
        }
    }
}
