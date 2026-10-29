//! The Java text of event fields, for the `Display` of events: `Double.toString`, `StringUtil.encodeChar`,
//! `TimeFormat`, `DayUtil` and the trade direction.

use std::fmt::{self, Display, Formatter, Write};
use std::ptr;
use std::sync::{Mutex, OnceLock};

use dxfeed_graal_sys as sys;

use crate::isolate::{self, call};

/// A `double` as Java's `Double.toString` prints it.
pub(super) struct JavaDouble(pub(super) f64);

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
        let (digits, exponent) = java_digits(a);
        let (first, rest) = digits.split_at(1);
        if (1e-3..1e7).contains(&a) {
            if exponent < 0 {
                let zeros = (-exponent - 1) as usize;
                write!(f, "0.{}{digits}", "0".repeat(zeros))
            } else {
                let point = exponent as usize + 1;
                if digits.len() > point {
                    write!(f, "{}.{}", &digits[..point], &digits[point..])
                } else {
                    write!(f, "{digits}{}.0", "0".repeat(point - digits.len()))
                }
            }
        } else {
            let rest = if rest.is_empty() { "0" } else { rest };
            write!(f, "{first}.{rest}E{exponent}")
        }
    }
}

/// The significant digits of a positive finite `a`, without trailing zeros, and the decimal exponent of the first one.
///
/// They are the shortest digits that read back as `a`, as Rust and Java 19+ choose them, with Java's rule that a value
/// whose shortest form has one digit uses the closest two-digit form (`5e-324` is `4.9E-324`).
fn java_digits(a: f64) -> (String, i32) {
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

/// A time in milliseconds since the Unix epoch, printed by the SDK's `TimeFormat.DEFAULT` in the default time zone,
/// with milliseconds or without; `0` prints as `0`.
///
/// While the process exits, or if the SDK fails, the time prints as its decimal milliseconds.
pub(super) struct Time {
    pub(super) millis: i64,
    pub(super) with_millis: bool,
}

impl Display for Time {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        if self.millis == 0 {
            return f.write_char('0');
        }
        match format_time(self.millis, self.with_millis) {
            Some(text) => f.write_str(&text),
            None => write!(f, "{}", self.millis),
        }
    }
}

/// The SDK's `TimeFormat.DEFAULT` and `TimeFormat.DEFAULT.withMillis()`; never released.
struct TimeFormats {
    default: *mut sys::dxfg_v4_time_format_t,
    with_millis: *mut sys::dxfg_v4_time_format_t,
}

// SAFETY: the handles are pointers to SDK memory usable from any attached thread, and Java's `TimeFormat` is
// thread-safe.
unsafe impl Send for TimeFormats {}
// SAFETY: as above.
unsafe impl Sync for TimeFormats {}

fn time_formats() -> Option<&'static TimeFormats> {
    static FORMATS: OnceLock<TimeFormats> = OnceLock::new();
    static INIT: Mutex<()> = Mutex::new(());
    if let Some(formats) = FORMATS.get() {
        return Some(formats);
    }
    // A failed call leaves the cell empty, so that the next call tries again.
    let _init = crate::lock(&INIT);
    if let Some(formats) = FORMATS.get() {
        return Some(formats);
    }
    let mut default = ptr::null_mut();
    // SAFETY: the out-pointer is valid for writes.
    call(|t| unsafe { sys::dxfg_v4_TimeFormat_DEFAULT(t, &mut default) }).ok()?;
    let mut with_millis = ptr::null_mut();
    // SAFETY: `default` is a live time format and the out-pointer is valid for writes.
    call(|t| unsafe { sys::dxfg_v4_TimeFormat_withMillis(t, default, &mut with_millis) }).ok()?;
    if default.is_null() || with_millis.is_null() {
        return None;
    }
    Some(FORMATS.get_or_init(|| TimeFormats { default, with_millis }))
}

fn format_time(millis: i64, with_millis: bool) -> Option<String> {
    if isolate::exiting() {
        return None;
    }
    let formats = time_formats()?;
    let format = if with_millis {
        formats.with_millis
    } else {
        formats.default
    };
    let mut text = ptr::null();
    // SAFETY: the time format is live and the out-pointer is valid for writes.
    call(|t| unsafe { sys::dxfg_v4_TimeFormat_format(t, format, millis, &mut text) }).ok()?;
    if text.is_null() {
        return None;
    }
    // SAFETY: the SDK wrote a NUL-terminated string that is released right after copying.
    let copy = unsafe { isolate::string(text) }.into_owned();
    // SAFETY: `text` came from dxfg_v4_TimeFormat_format and is released once.
    let _ = call(|t| unsafe { sys::dxfg_v4_String_release(t, text) });
    Some(copy)
}

/// The `yyyymmdd` number of a day id, the days since 1970-01-01, as Java's `DayUtil.getYearMonthDayByDayId` gives it:
/// proleptic Gregorian, negative for years before 0.
pub(super) fn year_month_day(day_id: i32) -> i32 {
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

/// The tick direction in bits 1-3 of trade flags, as Java's `Direction` names it; the unused codes are `UNDEFINED`.
pub(super) fn direction(flags: i32) -> &'static str {
    match (flags >> 1) & 7 {
        1 => "DOWN",
        2 => "ZERO_DOWN",
        3 => "ZERO",
        4 => "ZERO_UP",
        5 => "UP",
        _ => "UNDEFINED",
    }
}

/// Whether trade flags mark a trade in extended trading hours (bit 0).
pub(super) fn extended_trading_hours(flags: i32) -> bool {
    flags & 1 != 0
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
    fn directions_print_as_in_java() {
        let names: Vec<&str> = (0..8).map(|code| direction(code << 1)).collect();
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
        assert_eq!(direction(1 | 4 << 1), "ZERO_UP");
        assert!(extended_trading_hours(1));
        assert!(!extended_trading_hours(4 << 1));
    }

    #[test]
    fn zero_time_needs_no_sdk() {
        assert_eq!(
            Time {
                millis: 0,
                with_millis: true
            }
            .to_string(),
            "0"
        );
    }
}
