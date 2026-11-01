//! Time periods (`TimePeriod`) and the bounds of aggregation periods (`TimePeriodInfo`).

use std::fmt::{self, Display, Formatter, Write};
use std::str::FromStr;
use std::time::Duration;

use dxfeed_graal_sys as sys;

use crate::error::{Error, Result, c_string};
use crate::event::format::{JavaDouble, java_digits};
use crate::handle::Handle;
use crate::isolate::call;

/// A period of time in milliseconds, as Java's `TimePeriod`.
///
/// The text of a period, read by [`value_of`](Self::value_of) and written by `Display`, is that of Java: ISO 8601 such
/// as `PT1H30M`, a short form such as `1s`, `1.5s` or `1d2h` (case does not matter), or `inf` for
/// [`UNLIMITED`](Self::UNLIMITED).
///
/// ```no_run
/// # fn main() -> dxfeed_graal::Result<()> {
/// use dxfeed_graal::TimePeriod;
///
/// let period = TimePeriod::value_of("PT1H30M")?;
/// assert_eq!(period.get_time(), 5_400_000);
/// assert_eq!(period.to_string(), "PT1H30M0S");
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TimePeriod {
    millis: i64,
}

impl TimePeriod {
    /// The period of zero length (`ZERO`).
    pub const ZERO: TimePeriod = TimePeriod { millis: 0 };

    /// The period of the greatest length, `i64::MAX` milliseconds (`UNLIMITED`).
    pub const UNLIMITED: TimePeriod = TimePeriod { millis: i64::MAX };

    /// The period of `millis` milliseconds (`valueOf(long)`); as in Java, a negative length is kept.
    #[doc(alias = "valueOf")]
    pub const fn value_of_millis(millis: i64) -> TimePeriod {
        TimePeriod { millis }
    }

    /// The period of the text `value` (`valueOf(String)`), read by Java.
    ///
    /// Returns [`Error::Java`] for a text Java cannot read, such as `x` or an empty text.
    #[doc(alias = "valueOf")]
    pub fn value_of(value: &str) -> Result<TimePeriod> {
        let text = c_string(value, "the time period")?;
        // SAFETY: the text is NUL-terminated and the out-pointer is valid for writes.
        let handle = Handle::create(|t, out| unsafe { sys::dxfg_v4_TimePeriod_valueOfString(t, text.as_ptr(), out) })?;
        TimePeriod::from_sdk(&handle)
    }

    /// The period of the text `value`, or `default` for an empty text (`valueOf(String, TimePeriod)`).
    ///
    /// Returns [`Error::Java`] for a text Java cannot read.
    #[doc(alias = "valueOf")]
    pub fn value_of_or(value: &str, default: TimePeriod) -> Result<TimePeriod> {
        if value.is_empty() {
            Ok(default)
        } else {
            TimePeriod::value_of(value)
        }
    }

    /// The length in milliseconds (`getTime`).
    #[doc(alias = "getTime")]
    pub const fn get_time(self) -> i64 {
        self.millis
    }

    /// The length in whole seconds, rounded down and cut to 32 bits as in Java (`getSeconds`).
    #[doc(alias = "getSeconds")]
    pub const fn get_seconds(self) -> i32 {
        self.millis.div_euclid(1000) as i32
    }

    /// The length in nanoseconds, wrapping on overflow as in Java (`getNanos`).
    #[doc(alias = "getNanos")]
    pub const fn get_nanos(self) -> i64 {
        self.millis.wrapping_mul(1_000_000)
    }

    /// The Java `TimePeriod` of this period; the SDK refuses a negative one with [`Error::Java`].
    pub(crate) fn to_sdk(self) -> Result<Handle<sys::dxfg_v4_time_period_t>> {
        let millis = self.millis;
        // SAFETY: the out-pointer is valid for writes.
        Handle::create(|t, out| unsafe { sys::dxfg_v4_TimePeriod_valueOf(t, millis, out) })
    }

    /// The period of a Java `TimePeriod`.
    pub(crate) fn from_sdk(handle: &Handle<sys::dxfg_v4_time_period_t>) -> Result<TimePeriod> {
        let (p, mut millis) = (handle.as_ptr(), 0);
        // SAFETY: the period is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_TimePeriod_getTime(t, p, &mut millis) })?;
        Ok(TimePeriod { millis })
    }
}

impl Display for TimePeriod {
    /// Writes the text of Java's `toString`, such as `PT1H30M0S`, `PT1.5S` or `P1DT0S`.
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let mut v = self.millis;
        let millis = v % 1000;
        v /= 1000;
        let secs = v % 60;
        v /= 60;
        let mins = v % 60;
        v /= 60;
        let hours = v % 24;
        let days = v / 24;
        f.write_char('P')?;
        if days > 0 {
            write!(f, "{days}D")?;
        }
        f.write_char('T')?;
        if hours > 0 {
            write!(f, "{hours}H")?;
        }
        if mins > 0 {
            write!(f, "{mins}M")?;
        }
        if millis > 0 {
            write!(f, "{}", JavaDouble((secs * 1000 + millis) as f64 / 1000.0))?;
        } else {
            write!(f, "{secs}")?;
        }
        f.write_char('S')
    }
}

impl FromStr for TimePeriod {
    type Err = Error;

    /// Reads the text as [`TimePeriod::value_of`] does.
    fn from_str(s: &str) -> Result<TimePeriod> {
        TimePeriod::value_of(s)
    }
}

impl TryFrom<Duration> for TimePeriod {
    type Error = Error;

    /// The period of the whole milliseconds of `duration`; [`Error::InvalidArgument`] if they do not fit an `i64`.
    fn try_from(duration: Duration) -> Result<TimePeriod> {
        i64::try_from(duration.as_millis())
            .map(TimePeriod::value_of_millis)
            .map_err(|_| Error::InvalidArgument(format!("the period {duration:?} is too long")))
    }
}

impl TryFrom<TimePeriod> for Duration {
    type Error = Error;

    /// The duration of `period`; [`Error::InvalidArgument`] for a negative period.
    fn try_from(period: TimePeriod) -> Result<Duration> {
        u64::try_from(period.millis)
            .map(Duration::from_millis)
            .map_err(|_| Error::InvalidArgument(format!("the period {period} is negative")))
    }
}

/// The bounds of the aggregation periods that a server applies or that were applied to delivered events, in
/// milliseconds, as Java's `TimePeriodInfo`.
///
/// [`UNKNOWN`](Self::UNKNOWN), with negative bounds, means that no info is available.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TimePeriodInfo {
    min: i64,
    max: i64,
}

impl TimePeriodInfo {
    /// No info is available (`UNKNOWN`); both bounds are negative.
    pub const UNKNOWN: TimePeriodInfo = TimePeriodInfo { min: -1000, max: -1000 };

    /// The periods are known to be zero (`EMPTY`).
    pub const EMPTY: TimePeriodInfo = TimePeriodInfo { min: 0, max: 0 };

    /// The info of the bounds `min` and `max` in milliseconds (`valueOf`).
    ///
    /// Returns [`Error::InvalidArgument`] if a bound is negative or `min` is greater than `max`.
    #[doc(alias = "valueOf")]
    pub fn value_of(min: i64, max: i64) -> Result<TimePeriodInfo> {
        if min < 0 || max < 0 {
            return Err(Error::InvalidArgument(format!(
                "min and max must be non-negative, got: ({min},{max})"
            )));
        }
        if min > max {
            return Err(Error::InvalidArgument(format!(
                "min ({min}) must be less than or equal to max ({max})"
            )));
        }
        Ok(TimePeriodInfo { min, max })
    }

    /// The minimum period in milliseconds (`getMin`); negative for [`UNKNOWN`](Self::UNKNOWN).
    #[doc(alias = "getMin")]
    pub const fn get_min(self) -> i64 {
        self.min
    }

    /// The maximum period in milliseconds (`getMax`); negative for [`UNKNOWN`](Self::UNKNOWN).
    #[doc(alias = "getMax")]
    pub const fn get_max(self) -> i64 {
        self.max
    }

    /// Whether this is [`UNKNOWN`](Self::UNKNOWN) (`isUnknown`).
    #[doc(alias = "isUnknown")]
    pub const fn is_unknown(self) -> bool {
        self.min < 0 && self.max < 0
    }
}

impl Display for TimePeriodInfo {
    /// Writes the text of Java's `toString`: JSON with the periods in seconds, such as `{"min":1.5,"max":2}`;
    /// [`UNKNOWN`](TimePeriodInfo::UNKNOWN) is `{"min":-1,"max":-1}`.
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str("{\"min\":")?;
        write_seconds(f, self.min)?;
        f.write_str(",\"max\":")?;
        write_seconds(f, self.max)?;
        f.write_char('}')
    }
}

/// Writes milliseconds as seconds the way Java's `FastDoubleUtil.formatDoubleScale(millis / 1000.0, 3)` does: plain
/// below 1e10 seconds, such as `0.25` or `86400`, else as `1.5E10`; at most three fraction digits.
fn write_seconds(f: &mut Formatter<'_>, millis: i64) -> fmt::Result {
    if millis == 0 {
        return f.write_char('0');
    }
    if millis < 0 {
        f.write_char('-')?;
    }
    // 1e10 seconds; below it a double keeps every millisecond, so integer arithmetic gives the same digits.
    const PLAIN_LIMIT: u64 = 10_000_000_000_000;
    let magnitude = millis.unsigned_abs();
    if magnitude < PLAIN_LIMIT {
        write!(f, "{}", magnitude / 1000)?;
        let fraction = magnitude % 1000;
        if fraction != 0 {
            let digits = format!("{fraction:03}");
            write!(f, ".{}", digits.trim_end_matches('0'))?;
        }
        return Ok(());
    }
    // The shortest digits of the double. They never go past the milliseconds, the third fraction digit of the seconds,
    // as the text of the whole milliseconds reads back as the same double: Java's rounding to that digit keeps them.
    let (digits, exponent) = java_digits(magnitude as f64 / 1000.0);
    let (first, rest) = digits.split_at(1);
    if rest.is_empty() {
        write!(f, "{first}E{exponent}")
    } else {
        write!(f, "{first}.{rest}E{exponent}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn periods_print_as_in_java() {
        let text = |millis| TimePeriod::value_of_millis(millis).to_string();
        assert_eq!(text(0), "PT0S");
        assert_eq!(text(1000), "PT1S");
        assert_eq!(text(1500), "PT1.5S");
        assert_eq!(text(5_400_000), "PT1H30M0S");
        assert_eq!(text(86_400_000), "P1DT0S");
        assert_eq!(text(90_061_001), "P1DT1H1M1.001S");
        assert_eq!(TimePeriod::UNLIMITED.to_string(), "P106751991167DT7H12M55.807S");
        // Java's remainders keep the sign, and the negative parts are left out but the seconds.
        assert_eq!(text(-1500), "PT-1S");
        assert_eq!(text(-61_000), "PT-1S");
    }

    #[test]
    fn conversions_match_the_sdk() {
        // The SDK refuses a negative period, which Java's `valueOf(long)` keeps.
        assert!(matches!(TimePeriod::value_of_millis(-1).to_sdk(), Err(Error::Java(_))));
        for millis in [0, 1, 999, 1000, 1500, 5_400_000, i64::MAX / 1000, i64::MAX] {
            let period = TimePeriod::value_of_millis(millis);
            let handle = period.to_sdk().unwrap();
            let (p, mut seconds, mut nanos) = (handle.as_ptr(), 0, 0);
            // SAFETY: the period is live and the out-pointers are valid for writes.
            call(|t| unsafe { sys::dxfg_v4_TimePeriod_getSeconds(t, p, &mut seconds) }).unwrap();
            // SAFETY: as above.
            call(|t| unsafe { sys::dxfg_v4_TimePeriod_getNanos(t, p, &mut nanos) }).unwrap();
            assert_eq!((period.get_seconds(), period.get_nanos()), (seconds, nanos), "{millis}");
            assert_eq!(TimePeriod::from_sdk(&handle).unwrap(), period);
        }
    }

    #[test]
    fn periods_convert_as_in_java() {
        let period = TimePeriod::value_of_millis(-1500);
        assert_eq!(period.get_seconds(), -2);
        assert_eq!(period.get_nanos(), -1_500_000_000);
        assert_eq!(TimePeriod::UNLIMITED.get_seconds(), (i64::MAX / 1000) as i32);
        assert_eq!(TimePeriod::UNLIMITED.get_nanos(), i64::MAX.wrapping_mul(1_000_000));
        let duration = Duration::from_millis(123_456_789);
        let period = TimePeriod::try_from(duration).unwrap();
        assert_eq!(period.get_time(), 123_456_789);
        assert_eq!(Duration::try_from(period).unwrap(), duration);
        assert_eq!(
            TimePeriod::try_from(Duration::from_micros(1999)).unwrap(),
            TimePeriod::value_of_millis(1)
        );
        assert!(matches!(
            TimePeriod::try_from(Duration::MAX),
            Err(Error::InvalidArgument(_))
        ));
        assert!(matches!(
            Duration::try_from(TimePeriod::value_of_millis(-1)),
            Err(Error::InvalidArgument(_))
        ));
    }

    #[test]
    fn infos_are_checked_and_printed_as_in_java() {
        assert!(TimePeriodInfo::UNKNOWN.is_unknown());
        assert!(!TimePeriodInfo::EMPTY.is_unknown());
        assert_eq!(TimePeriodInfo::value_of(0, 0).unwrap(), TimePeriodInfo::EMPTY);
        let info = TimePeriodInfo::value_of(1500, 2000).unwrap();
        assert_eq!((info.get_min(), info.get_max()), (1500, 2000));
        assert_eq!(info.to_string(), r#"{"min":1.5,"max":2}"#);
        assert_eq!(TimePeriodInfo::UNKNOWN.to_string(), r#"{"min":-1,"max":-1}"#);
        assert_eq!(
            TimePeriodInfo::value_of(250, 86_400_000).unwrap().to_string(),
            r#"{"min":0.25,"max":86400}"#
        );
        let message = |min, max| match TimePeriodInfo::value_of(min, max) {
            Err(Error::InvalidArgument(message)) => message,
            other => panic!("{other:?}"),
        };
        assert_eq!(message(-1, 5), "min and max must be non-negative, got: (-1,5)");
        assert_eq!(message(5, 1), "min (5) must be less than or equal to max (1)");
    }

    #[test]
    fn large_seconds_use_the_exponent() {
        let text = |millis| {
            TimePeriodInfo {
                min: millis,
                max: millis,
            }
            .to_string()
        };
        assert_eq!(text(10_000_000_000_000), r#"{"min":1E10,"max":1E10}"#);
        assert_eq!(
            text(12_345_678_901_234_567),
            r#"{"min":1.2345678901234568E13,"max":1.2345678901234568E13}"#
        );
        assert_eq!(text(99_999_999_999_999_999), r#"{"min":1E14,"max":1E14}"#);
        assert_eq!(text(-15_000_000_000_000), r#"{"min":-1.5E10,"max":-1.5E10}"#);
    }
}
