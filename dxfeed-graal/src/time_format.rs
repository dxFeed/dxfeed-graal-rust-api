//! Times parsed and formatted as the other dxFeed APIs do it (`TimeFormat`).

use std::ptr;
use std::sync::{Mutex, OnceLock};

use dxfeed_graal_sys as sys;

use crate::error::{Error, JavaException, Result, c_string};
use crate::isolate::{self, Thread, call};

/// Parses and formats times as the Java `TimeFormat` of the SDK does; times are milliseconds since the Unix epoch.
///
/// [`DEFAULT`](Self::DEFAULT) uses the default time zone of the process, [`GMT`](Self::GMT) the GMT zone. The text
/// formats, such as `20070101-123456`, `20070101-123456.123`, `2005-12-31 21:00:00.123+03:00` or `2007-11-02Z`, are
/// those of Java's `TimeFormat.parse`; a number is milliseconds since the epoch.
///
/// ```no_run
/// # fn main() -> dxfeed_graal::Result<()> {
/// use dxfeed_graal::TimeFormat;
///
/// let time = TimeFormat::GMT.parse("20070101-123456")?;
/// assert_eq!(TimeFormat::GMT.format(time)?, "20070101-123456");
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TimeFormat {
    gmt: bool,
    millis: bool,
    time_zone: bool,
    full_iso: bool,
}

impl TimeFormat {
    /// The format of the default time zone (`TimeFormat.DEFAULT`).
    pub const DEFAULT: TimeFormat = TimeFormat {
        gmt: false,
        millis: false,
        time_zone: false,
        full_iso: false,
    };

    /// The format of the GMT time zone (`TimeFormat.GMT`).
    pub const GMT: TimeFormat = TimeFormat {
        gmt: true,
        ..TimeFormat::DEFAULT
    };

    /// This format with milliseconds in the formatted text (`withMillis`).
    #[doc(alias = "withMillis")]
    pub const fn with_millis(self) -> TimeFormat {
        TimeFormat { millis: true, ..self }
    }

    /// This format with the time zone in the formatted text (`withTimeZone`).
    #[doc(alias = "withTimeZone")]
    pub const fn with_time_zone(self) -> TimeFormat {
        TimeFormat {
            time_zone: true,
            ..self
        }
    }

    /// This format with the full ISO 8601 text, such as `2007-01-01T12:34:56.000+03:00` (`asFullIso`).
    #[doc(alias = "asFullIso")]
    pub const fn as_full_iso(self) -> TimeFormat {
        TimeFormat { full_iso: true, ..self }
    }

    /// Parses `value` to milliseconds since the Unix epoch (`parse`); a text without a time zone is in the zone of the
    /// format.
    ///
    /// Returns [`Error::Java`] for a text Java cannot parse.
    pub fn parse(self, value: &str) -> Result<i64> {
        let text = c_string(value, "the time")?;
        let format = self.sdk()?;
        let mut time = 0;
        // SAFETY: the time format is live, the text is NUL-terminated and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_TimeFormat_parse(t, format, text.as_ptr(), &mut time) })?;
        Ok(time)
    }

    /// Formats milliseconds since the Unix epoch (`format`).
    pub fn format(self, millis: i64) -> Result<String> {
        let format = self.sdk()?;
        let mut text = ptr::null();
        // SAFETY: the time format is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_TimeFormat_format(t, format, millis, &mut text) })?;
        if text.is_null() {
            return Err(Error::Java(JavaException::null_result()));
        }
        // SAFETY: the SDK wrote a NUL-terminated string that is released right after copying.
        let copy = unsafe { isolate::string(text) }.into_owned();
        // SAFETY: `text` came from dxfg_v4_TimeFormat_format and is released once.
        crate::error_manager::report_release("TimeFormat", call(|t| unsafe { sys::dxfg_v4_String_release(t, text) }));
        Ok(copy)
    }

    /// The SDK format, created on first use and kept for the process, as are the formats it is derived from.
    fn sdk(self) -> Result<*mut sys::dxfg_v4_time_format_t> {
        static FORMATS: [OnceLock<SdkFormat>; 16] = [const { OnceLock::new() }; 16];
        static INIT: Mutex<()> = Mutex::new(());
        let cell = &FORMATS[self.index()];
        if let Some(format) = cell.get() {
            return Ok(format.0);
        }
        // The format is derived from the one without its last option, created first and outside the lock.
        type Derive =
            unsafe extern "C" fn(Thread, *mut sys::dxfg_v4_time_format_t, *mut *mut sys::dxfg_v4_time_format_t) -> i32;
        let derive: Option<(TimeFormat, Derive)> = if self.full_iso {
            Some((
                TimeFormat {
                    full_iso: false,
                    ..self
                },
                sys::dxfg_v4_TimeFormat_asFullIso,
            ))
        } else if self.time_zone {
            Some((
                TimeFormat {
                    time_zone: false,
                    ..self
                },
                sys::dxfg_v4_TimeFormat_withTimeZone,
            ))
        } else if self.millis {
            Some((TimeFormat { millis: false, ..self }, sys::dxfg_v4_TimeFormat_withMillis))
        } else {
            None
        };
        let parent = match derive {
            Some((parent, _)) => parent.sdk()?,
            None => ptr::null_mut(),
        };
        let format = crate::get_or_try_init(cell, &INIT, || {
            let mut format = ptr::null_mut();
            call(|t| match derive {
                // SAFETY: the parent format is live and the out-pointer is valid for writes.
                Some((_, f)) => unsafe { f(t, parent, &mut format) },
                // SAFETY: the out-pointer is valid for writes.
                None if self.gmt => unsafe { sys::dxfg_v4_TimeFormat_GMT(t, &mut format) },
                // SAFETY: the out-pointer is valid for writes.
                None => unsafe { sys::dxfg_v4_TimeFormat_DEFAULT(t, &mut format) },
            })?;
            if format.is_null() {
                return Err(Error::Java(JavaException::null_result()));
            }
            Ok(SdkFormat(format))
        })?;
        Ok(format.0)
    }

    fn index(self) -> usize {
        usize::from(self.gmt)
            | usize::from(self.millis) << 1
            | usize::from(self.time_zone) << 2
            | usize::from(self.full_iso) << 3
    }
}

/// A `TimeFormat` of the SDK; never released.
struct SdkFormat(*mut sys::dxfg_v4_time_format_t);

// SAFETY: the handle is a pointer to SDK memory usable from any attached thread, and Java's `TimeFormat` is
// thread-safe.
unsafe impl Send for SdkFormat {}
// SAFETY: as above.
unsafe impl Sync for SdkFormat {}
