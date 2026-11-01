//! Time zones (`java.util.TimeZone`).

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use dxfeed_graal_sys as sys;

use crate::error::{Result, c_string};
use crate::handle::Handle;
use crate::isolate::{self, call};

/// A time zone of Java, as `java.util.TimeZone`; offsets are in milliseconds and times in milliseconds since the Unix
/// epoch.
///
/// Unlike Java's, the zone cannot change: it has no `setID` or `setRawOffset`. A zone of a fixed offset has an
/// identifier such as `GMT+05:30`. Zones are equal when their identifiers are.
///
/// ```no_run
/// # fn main() -> dxfeed_graal::Result<()> {
/// use dxfeed_graal::TimeZone;
///
/// let zone = TimeZone::get_time_zone("America/New_York")?;
/// assert_eq!(zone.get_raw_offset()?, -5 * 3_600_000);
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct TimeZone {
    handle: Arc<Handle<sys::dxfg_v4_time_zone_t>>,
    id: String,
}

/// An `int` getter of a time zone.
macro_rules! int_getter {
    ($(#[$doc:meta])* $name:ident: $sdk:ident) => {
        $(#[$doc])*
        pub fn $name(&self) -> Result<i32> {
            let (zone, mut value) = (self.as_ptr(), 0);
            // SAFETY: the time zone is live and the out-pointer is valid for writes.
            call(|t| unsafe { sys::$sdk(t, zone, &mut value) })?;
            Ok(value)
        }
    };
}

/// A `boolean` getter of a time zone.
macro_rules! bool_getter {
    ($(#[$doc:meta])* $name:ident: $sdk:ident) => {
        $(#[$doc])*
        pub fn $name(&self) -> Result<bool> {
            let (zone, mut value) = (self.as_ptr(), 0);
            // SAFETY: the time zone is live and the out-pointer is valid for writes.
            call(|t| unsafe { sys::$sdk(t, zone, &mut value) })?;
            Ok(value != 0)
        }
    };
}

impl TimeZone {
    /// The style of a short display name, such as `GMT-05:00` (`SHORT`).
    pub const SHORT: i32 = 0;

    /// The style of a long display name, such as `New York (+0)` (`LONG`).
    pub const LONG: i32 = 1;

    /// The time zone of the identifier `id`, such as `America/New_York` or `GMT+03:00` (`getTimeZone`); an unknown
    /// identifier gives the GMT zone, as in Java.
    #[doc(alias = "getTimeZone")]
    pub fn get_time_zone(id: &str) -> Result<TimeZone> {
        let id = c_string(id, "the time zone ID")?;
        // SAFETY: the identifier is NUL-terminated and the out-pointer is valid for writes.
        TimeZone::from_handle(Handle::create(|t, out| unsafe {
            sys::dxfg_v4_TimeZone_getTimeZone(t, id.as_ptr(), out)
        })?)
    }

    /// The default time zone of the process (`getDefault`).
    #[doc(alias = "getDefault")]
    pub fn get_default() -> Result<TimeZone> {
        // SAFETY: the out-pointer is valid for writes.
        TimeZone::from_handle(Handle::create(|t, out| unsafe {
            sys::dxfg_v4_TimeZone_getDefault(t, out)
        })?)
    }

    /// The identifier of the zone, such as `America/New_York` (`getID`).
    #[doc(alias = "getID")]
    pub fn get_id(&self) -> &str {
        &self.id
    }

    /// The long name of the standard time of the zone in the default locale (`getDisplayName`).
    ///
    /// The locale data of the SDK has generic names only, such as `New York (+0)` for the standard time of
    /// `America/New_York` and `New York (+1)` for its daylight saving time, where Java has `Eastern Standard Time`.
    #[doc(alias = "getDisplayName")]
    pub fn get_display_name(&self) -> Result<String> {
        let zone = self.as_ptr();
        // SAFETY: the time zone is live and the out-pointer is valid for writes.
        isolate::out_string("TimeZone", |t, name| unsafe {
            sys::dxfg_v4_TimeZone_getDisplayName(t, zone, name)
        })
    }

    /// The name of the zone in the default locale, of its daylight saving time if `daylight`, in the style
    /// [`SHORT`](Self::SHORT) or [`LONG`](Self::LONG) (`getDisplayName(boolean, int)`).
    ///
    /// Returns [`Error::Java`](crate::Error::Java) for another style.
    #[doc(alias = "getDisplayName")]
    pub fn get_display_name_with_style(&self, daylight: bool, style: i32) -> Result<String> {
        let zone = self.as_ptr();
        // SAFETY: the time zone is live and the out-pointer is valid for writes.
        isolate::out_string("TimeZone", |t, name| unsafe {
            sys::dxfg_v4_TimeZone_getDisplayNameWithStyle(t, zone, i32::from(daylight), style, name)
        })
    }

    int_getter!(
        /// The time added to the standard time in the daylight saving time, in milliseconds; zero if the zone has no
        /// daylight saving time (`getDSTSavings`).
        #[doc(alias = "getDSTSavings")]
        get_dst_savings: dxfg_v4_TimeZone_getDSTSavings
    );

    bool_getter!(
        /// Whether the zone has daylight saving time now or later (`useDaylightTime`).
        #[doc(alias = "useDaylightTime")]
        use_daylight_time: dxfg_v4_TimeZone_useDaylightTime
    );

    bool_getter!(
        /// Whether the zone is in daylight saving time now or has it later (`observesDaylightTime`).
        #[doc(alias = "observesDaylightTime")]
        observes_daylight_time: dxfg_v4_TimeZone_observesDaylightTime
    );

    int_getter!(
        /// The offset of the standard time from UTC, in milliseconds (`getRawOffset`).
        #[doc(alias = "getRawOffset")]
        get_raw_offset: dxfg_v4_TimeZone_getRawOffset
    );

    /// The offset from UTC at the time `date`, with the daylight saving time, in milliseconds (`getOffset(long)`).
    #[doc(alias = "getOffset")]
    pub fn get_offset(&self, date: i64) -> Result<i32> {
        let (zone, mut offset) = (self.as_ptr(), 0);
        // SAFETY: the time zone is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_TimeZone_getOffset(t, zone, date, &mut offset) })?;
        Ok(offset)
    }

    /// The offset from UTC at a local standard time given by its fields as in Java's `Calendar`, in milliseconds
    /// (`getOffset(int, int, int, int, int, int)`): `era` 1 for AD, `month` from 0 for January, `day_of_week` from 1
    /// for Sunday and `millis` of the day.
    ///
    /// Returns [`Error::Java`](crate::Error::Java) for a field out of its range.
    #[doc(alias = "getOffset")]
    pub fn get_offset_by_calendar_fields(
        &self,
        era: i32,
        year: i32,
        month: i32,
        day: i32,
        day_of_week: i32,
        millis: i32,
    ) -> Result<i32> {
        let (zone, mut offset) = (self.as_ptr(), 0);
        // SAFETY: the time zone is live and the out-pointer is valid for writes.
        call(|t| unsafe {
            sys::dxfg_v4_TimeZone_getOffsetByCalendarFields(
                t,
                zone,
                era,
                year,
                month,
                day,
                day_of_week,
                millis,
                &mut offset,
            )
        })?;
        Ok(offset)
    }

    /// Whether `other` has the same raw offset and daylight saving time rules, whatever its identifier
    /// (`hasSameRules`).
    #[doc(alias = "hasSameRules")]
    pub fn has_same_rules(&self, other: &TimeZone) -> Result<bool> {
        let (zone, other, mut same) = (self.as_ptr(), other.as_ptr(), 0);
        // SAFETY: both time zones are live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_TimeZone_hasSameRules(t, zone, other, &mut same) })?;
        Ok(same != 0)
    }

    /// Whether the time `date` is in the daylight saving time of the zone (`inDaylightTime`).
    #[doc(alias = "inDaylightTime")]
    pub fn in_daylight_time(&self, date: i64) -> Result<bool> {
        let (zone, mut daylight) = (self.as_ptr(), 0);
        // SAFETY: the time zone is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_TimeZone_inDaylightTime(t, zone, date, &mut daylight) })?;
        Ok(daylight != 0)
    }

    /// The zone of a handle that the SDK returned, with its identifier read once.
    pub(crate) fn from_handle(handle: Handle<sys::dxfg_v4_time_zone_t>) -> Result<TimeZone> {
        let zone = handle.as_ptr();
        // SAFETY: the time zone is live and the out-pointer is valid for writes.
        let id = isolate::out_string("TimeZone", |t, id| unsafe { sys::dxfg_v4_TimeZone_getID(t, zone, id) })?;
        Ok(TimeZone {
            handle: Arc::new(handle),
            id,
        })
    }

    /// The raw pointer, valid while `self` lives.
    pub(crate) fn as_ptr(&self) -> *mut sys::dxfg_v4_time_zone_t {
        self.handle.as_ptr()
    }
}

impl PartialEq for TimeZone {
    fn eq(&self, other: &TimeZone) -> bool {
        self.id == other.id
    }
}

impl Eq for TimeZone {}

impl Hash for TimeZone {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl fmt::Debug for TimeZone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("TimeZone").field(&self.id).finish()
    }
}
