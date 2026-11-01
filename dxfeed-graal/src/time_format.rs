//! Times parsed and formatted as the other dxFeed APIs do it (`TimeFormat`).

use std::collections::{HashMap, HashSet};
use std::ptr;
use std::sync::{Mutex, OnceLock};

use dxfeed_graal_sys as sys;

use crate::TimeZone;
use crate::error::{Error, JavaException, Result, c_string};
use crate::handle::Handle;
use crate::isolate::{self, Thread, call};

/// Parses and formats times as the Java `TimeFormat` of the SDK does; times are milliseconds since the Unix epoch.
///
/// [`DEFAULT`](Self::DEFAULT) uses the default time zone of the process, [`GMT`](Self::GMT) the GMT zone and
/// [`get_instance`](Self::get_instance) a given zone. The text formats, such as `20070101-123456`, `20070101-123456.123`, `2005-12-31 21:00:00.123+03:00` or `2007-11-02Z`, are
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
    /// The identifier of the time zone of a format of [`get_instance`](Self::get_instance).
    zone: Option<&'static str>,
    gmt: bool,
    millis: bool,
    time_zone: bool,
    full_iso: bool,
}

impl TimeFormat {
    /// The format of the default time zone (`TimeFormat.DEFAULT`).
    pub const DEFAULT: TimeFormat = TimeFormat {
        zone: None,
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

    /// The format of the time zone `zone` (`getInstance`).
    ///
    /// The format refers to the zone by its identifier, as a [`TimeZone`] cannot change; every zone of this crate has
    /// an identifier that Java finds again.
    #[doc(alias = "getInstance")]
    pub fn get_instance(zone: &TimeZone) -> TimeFormat {
        static IDS: Mutex<Option<HashSet<&'static str>>> = Mutex::new(None);
        let mut ids = crate::lock(&IDS);
        let ids = ids.get_or_insert_with(HashSet::new);
        // The identifiers are kept for the process, as the formats made from them are; there are few distinct zones.
        let id = match ids.get(zone.get_id()) {
            Some(id) => *id,
            None => {
                let id: &'static str = Box::leak(zone.get_id().into());
                ids.insert(id);
                id
            }
        };
        TimeFormat {
            zone: Some(id),
            ..TimeFormat::DEFAULT
        }
    }

    /// The time zone of the format (`getTimeZone`).
    #[doc(alias = "getTimeZone")]
    pub fn get_time_zone(self) -> Result<TimeZone> {
        let format = self.sdk()?;
        // SAFETY: the time format is live and the out-pointer is valid for writes.
        let handle = Handle::create(|t, out| unsafe { sys::dxfg_v4_TimeFormat_getTimeZone(t, format, out) })?;
        TimeZone::from_handle(handle)
    }

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
        // SAFETY: the time format is live and the out-pointer is valid for writes.
        isolate::out_string("TimeFormat", |t, text| unsafe {
            sys::dxfg_v4_TimeFormat_format(t, format, millis, text)
        })
    }

    /// The SDK format, created on first use and kept for the process, as are the formats it is derived from.
    fn sdk(self) -> Result<*mut sys::dxfg_v4_time_format_t> {
        static FORMATS: [OnceLock<SdkFormat>; 16] = [const { OnceLock::new() }; 16];
        static INIT: Mutex<()> = Mutex::new(());
        static ZONED: Mutex<Option<HashMap<TimeFormat, SdkFormat>>> = Mutex::new(None);
        let cell = &FORMATS[self.index()];
        if self.zone.is_none()
            && let Some(format) = cell.get()
        {
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
        let create = || {
            let zone = match (derive, self.zone) {
                (None, Some(id)) => Some(TimeZone::get_time_zone(id)?),
                _ => None,
            };
            let mut format = ptr::null_mut();
            call(|t| match (derive, &zone) {
                // SAFETY: the parent format is live and the out-pointer is valid for writes.
                (Some((_, f)), _) => unsafe { f(t, parent, &mut format) },
                // SAFETY: the time zone is live and the out-pointer is valid for writes.
                (None, Some(zone)) => unsafe { sys::dxfg_v4_TimeFormat_getInstance(t, zone.as_ptr(), &mut format) },
                // SAFETY: the out-pointer is valid for writes.
                (None, None) if self.gmt => unsafe { sys::dxfg_v4_TimeFormat_GMT(t, &mut format) },
                // SAFETY: the out-pointer is valid for writes.
                (None, None) => unsafe { sys::dxfg_v4_TimeFormat_DEFAULT(t, &mut format) },
            })?;
            if format.is_null() {
                return Err(Error::Java(JavaException::null_result()));
            }
            Ok(SdkFormat(format))
        };
        if self.zone.is_none() {
            return Ok(crate::get_or_try_init(cell, &INIT, create)?.0);
        }
        let mut zoned = crate::lock(&ZONED);
        let zoned = zoned.get_or_insert_with(HashMap::new);
        if let Some(format) = zoned.get(&self) {
            return Ok(format.0);
        }
        let format = create()?;
        Ok(zoned.entry(self).or_insert(format).0)
    }

    /// The index in the formats of the zones that need no identifier.
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
