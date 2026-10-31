//! Trading schedules (`com.dxfeed.schedule`): the trading days of an instrument and their sessions.
//!
//! A [`Schedule`] comes from the trading hours of an [`InstrumentProfile`] or from a schedule definition. Its
//! [`Day`]s hold [`Session`]s of a [`SessionType`]; [`DayFilter`] and [`SessionFilter`] select them in searches.
//!
//! ```no_run
//! use dxfeed_graal::schedule::{Schedule, SessionFilter};
//!
//! let schedule = Schedule::get_instance_by_definition(
//!     "NewYorkETH(name=US ETH;tz=America/New_York;td=12345;de=+0000;0=p04000930r09301600a16002000)",
//! )?;
//! let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
//! let session = schedule.get_nearest_session_by_time(now, SessionFilter::Trading)?;
//! println!("{session} in {}", session.day()?);
//! # Ok::<(), dxfeed_graal::Error>(())
//! ```

use std::ffi::c_char;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ptr;

use dxfeed_graal_sys as sys;

use crate::error::{Error, Result, c_string};
use crate::handle::Handle;
use crate::ipf::InstrumentProfile;
use crate::ipf::profile::take_string;
use crate::isolate::{self, call};

/// The type of a session (`SessionType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionType {
    /// No trading: the time before, between or after the trading sessions, or a non-trading day.
    NoTrading,
    /// The pre-market session before the regular one.
    PreMarket,
    /// The regular trading session.
    Regular,
    /// The after-market session after the regular one.
    AfterMarket,
}

impl SessionType {
    fn from_sys(session_type: sys::dxfg_v4_session_type_t) -> Result<Self> {
        Ok(match session_type {
            sys::DXFG_SESSION_TYPE_NO_TRADING => SessionType::NoTrading,
            sys::DXFG_SESSION_TYPE_PRE_MARKET => SessionType::PreMarket,
            sys::DXFG_SESSION_TYPE_REGULAR => SessionType::Regular,
            sys::DXFG_SESSION_TYPE_AFTER_MARKET => SessionType::AfterMarket,
            other => return Err(Error::InvalidArgument(format!("unknown session type {other}"))),
        })
    }

    /// Whether trading is allowed in the sessions of this type: all but [`NoTrading`](Self::NoTrading)
    /// (`isTrading`).
    #[doc(alias = "isTrading")]
    pub fn is_trading(self) -> bool {
        self != SessionType::NoTrading
    }
}

impl fmt::Display for SessionType {
    /// The name of the type in Java, such as `PRE_MARKET`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            SessionType::NoTrading => "NO_TRADING",
            SessionType::PreMarket => "PRE_MARKET",
            SessionType::Regular => "REGULAR",
            SessionType::AfterMarket => "AFTER_MARKET",
        })
    }
}

/// A predefined filter of days for the searches of [`Day`] (`DayFilter`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DayFilter {
    /// Any day (`DayFilter.ANY`).
    Any,
    /// Trading days (`DayFilter.TRADING`).
    Trading,
    /// Non-trading days (`DayFilter.NON_TRADING`).
    NonTrading,
    /// Holidays (`DayFilter.HOLIDAY`).
    Holiday,
    /// Short days (`DayFilter.SHORT_DAY`).
    ShortDay,
    /// Mondays (`DayFilter.MONDAY`).
    Monday,
    /// Tuesdays (`DayFilter.TUESDAY`).
    Tuesday,
    /// Wednesdays (`DayFilter.WEDNESDAY`).
    Wednesday,
    /// Thursdays (`DayFilter.THURSDAY`).
    Thursday,
    /// Fridays (`DayFilter.FRIDAY`).
    Friday,
    /// Saturdays (`DayFilter.SATURDAY`).
    Saturday,
    /// Sundays (`DayFilter.SUNDAY`).
    Sunday,
    /// Monday to Friday (`DayFilter.WEEK_DAY`).
    WeekDay,
    /// Saturday and Sunday (`DayFilter.WEEK_END`).
    WeekEnd,
}

impl DayFilter {
    fn handle(self) -> Result<Handle<sys::dxfg_v4_day_filter_t>> {
        let filter = match self {
            DayFilter::Any => sys::DXFG_DAY_FILTER_ANY,
            DayFilter::Trading => sys::DXFG_DAY_FILTER_TRADING,
            DayFilter::NonTrading => sys::DXFG_DAY_FILTER_NON_TRADING,
            DayFilter::Holiday => sys::DXFG_DAY_FILTER_HOLIDAY,
            DayFilter::ShortDay => sys::DXFG_DAY_FILTER_SHORT_DAY,
            DayFilter::Monday => sys::DXFG_DAY_FILTER_MONDAY,
            DayFilter::Tuesday => sys::DXFG_DAY_FILTER_TUESDAY,
            DayFilter::Wednesday => sys::DXFG_DAY_FILTER_WEDNESDAY,
            DayFilter::Thursday => sys::DXFG_DAY_FILTER_THURSDAY,
            DayFilter::Friday => sys::DXFG_DAY_FILTER_FRIDAY,
            DayFilter::Saturday => sys::DXFG_DAY_FILTER_SATURDAY,
            DayFilter::Sunday => sys::DXFG_DAY_FILTER_SUNDAY,
            DayFilter::WeekDay => sys::DXFG_DAY_FILTER_WEEK_DAY,
            DayFilter::WeekEnd => sys::DXFG_DAY_FILTER_WEEK_END,
        };
        // SAFETY: the out-pointer is valid for writes.
        Handle::create(|t, out| unsafe { sys::dxfg_v4_DayFilter_getInstance(t, filter, out) })
    }
}

/// A predefined filter of sessions for the searches of [`Day`], [`Session`] and [`Schedule`] (`SessionFilter`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionFilter {
    /// Any session (`SessionFilter.ANY`).
    Any,
    /// Trading sessions (`SessionFilter.TRADING`).
    Trading,
    /// Non-trading sessions (`SessionFilter.NON_TRADING`).
    NonTrading,
    /// Sessions of the type [`SessionType::NoTrading`] (`SessionFilter.NO_TRADING`).
    NoTrading,
    /// Sessions of the type [`SessionType::PreMarket`] (`SessionFilter.PRE_MARKET`).
    PreMarket,
    /// Sessions of the type [`SessionType::Regular`] (`SessionFilter.REGULAR`).
    Regular,
    /// Sessions of the type [`SessionType::AfterMarket`] (`SessionFilter.AFTER_MARKET`).
    AfterMarket,
}

impl SessionFilter {
    fn handle(self) -> Result<Handle<sys::dxfg_v4_session_filter_t>> {
        let filter = match self {
            SessionFilter::Any => sys::DXFG_SESSION_FILTER_ANY,
            SessionFilter::Trading => sys::DXFG_SESSION_FILTER_TRADING,
            SessionFilter::NonTrading => sys::DXFG_SESSION_FILTER_NON_TRADING,
            SessionFilter::NoTrading => sys::DXFG_SESSION_FILTER_NO_TRADING,
            SessionFilter::PreMarket => sys::DXFG_SESSION_FILTER_PRE_MARKET,
            SessionFilter::Regular => sys::DXFG_SESSION_FILTER_REGULAR,
            SessionFilter::AfterMarket => sys::DXFG_SESSION_FILTER_AFTER_MARKET,
        };
        // SAFETY: the out-pointer is valid for writes.
        Handle::create(|t, out| unsafe { sys::dxfg_v4_SessionFilter_getInstance(t, filter, out) })
    }
}

/// A getter of an SDK object that writes a value to its out-pointer.
macro_rules! getter {
    ($(#[$doc:meta])* $name:ident: $sys:ident -> $ty:ty = $init:expr $(, $map:expr)?) => {
        $(#[$doc])*
        pub fn $name(&self) -> Result<$ty> {
            let mut value = $init;
            let h = self.handle.as_ptr();
            // SAFETY: the handle is live and the out-pointer is valid for writes.
            call(|t| unsafe { sys::$sys(t, h, &mut value) })?;
            Ok(getter!(@map value $(, $map)?))
        }
    };
    (@map $value:ident) => { $value };
    (@map $value:ident, $map:expr) => { $map($value) };
}

/// A getter of an SDK object that writes a string to its out-pointer.
macro_rules! string_getter {
    ($(#[$doc:meta])* $name:ident: $sys:ident) => {
        $(#[$doc])*
        pub fn $name(&self) -> Result<String> {
            let mut text: *mut c_char = ptr::null_mut();
            let h = self.handle.as_ptr();
            // SAFETY: the handle is live and the out-pointer is valid for writes.
            call(|t| unsafe { sys::$sys(t, h, (&raw mut text).cast()) })?;
            // SAFETY: the SDK wrote a NUL-terminated string or NULL, which is ours to release.
            Ok(unsafe { take_string(text) })
        }
    };
}

/// `Display`, `PartialEq` and `Hash` of an SDK object by Java's `toString`, `equals` and `hashCode`.
macro_rules! java_object {
    ($name:ident, $to_string:ident, $equals:ident, $hash_code:ident) => {
        impl fmt::Display for $name {
            /// The text of Java's `toString`.
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                let mut text = ptr::null();
                let h = self.handle.as_ptr();
                // SAFETY: the handle is live and the out-pointer is valid for writes.
                match call(|t| unsafe { sys::$to_string(t, h, &mut text) }) {
                    // SAFETY: the SDK wrote a NUL-terminated string or NULL, which is ours to release.
                    Ok(()) => f.write_str(&unsafe { take_string(text) }),
                    Err(error) => write!(f, "{}(<{error}>)", stringify!($name)),
                }
            }
        }

        impl PartialEq for $name {
            /// Java's `equals`; `false` if the SDK fails.
            fn eq(&self, other: &Self) -> bool {
                let mut equal = 0;
                let (a, b) = (self.handle.as_ptr(), other.handle.as_ptr());
                // SAFETY: both handles are live and the out-pointer is valid for writes.
                call(|t| unsafe { sys::$equals(t, a, b, &mut equal) }).is_ok() && equal != 0
            }
        }

        impl Eq for $name {}

        impl Hash for $name {
            /// Java's `hashCode`; `0` if the SDK fails.
            fn hash<H: Hasher>(&self, state: &mut H) {
                let mut hash = 0;
                let h = self.handle.as_ptr();
                // SAFETY: the handle is live and the out-pointer is valid for writes.
                let _ = call(|t| unsafe { sys::$hash_code(t, h, &mut hash) });
                hash.hash(state);
            }
        }
    };
}

/// The trading schedule of an instrument: its days and their sessions, in the time zone of the schedule
/// (`Schedule`).
///
/// Times are in milliseconds since the Unix epoch. A schedule comes from the trading hours of a profile, such as
/// `NewYorkETH()`, or from a definition, such as
/// `NewYorkETH(name=US ETH;tz=America/New_York;td=12345;de=+0000;0=p04000930r09301600a16002000)`; the defaults
/// give the venues, holidays and short days that definitions refer to, and [`set_defaults`](Self::set_defaults)
/// or [`download_defaults`](Self::download_defaults) update them.
pub struct Schedule {
    handle: Handle<sys::dxfg_v4_schedule_t>,
}

impl fmt::Debug for Schedule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("Schedule");
        match self.name() {
            Ok(name) => debug.field("name", &name),
            Err(_) => debug.field("name", &format_args!("<error>")),
        };
        match self.definition() {
            Ok(definition) => debug.field("definition", &definition),
            Err(_) => debug.field("definition", &format_args!("<error>")),
        };
        debug.finish()
    }
}

impl Schedule {
    /// The default schedule of a profile, defined by its trading hours (`getInstance(InstrumentProfile)`).
    ///
    /// Returns an error if the trading hours define no schedule.
    #[doc(alias = "getInstance")]
    pub fn get_instance(profile: &InstrumentProfile) -> Result<Self> {
        let mut profile = profile.to_sys()?;
        let p = &raw mut profile.data;
        // SAFETY: the profile data is live; the out-pointer is valid for writes.
        let handle = Handle::create(|t, out| unsafe { sys::dxfg_v4_Schedule_getInstance(t, p, out) })?;
        Ok(Schedule { handle })
    }

    /// The schedule of a profile at a trading venue, one of [`get_trading_venues`](Self::get_trading_venues)
    /// (`getInstance(InstrumentProfile, String)`).
    ///
    /// Returns an error if the trading hours define no schedule for the venue.
    #[doc(alias = "getInstance")]
    pub fn get_instance_with_venue(profile: &InstrumentProfile, venue: &str) -> Result<Self> {
        let venue = c_string(venue, "the venue")?;
        let mut profile = profile.to_sys()?;
        let p = &raw mut profile.data;
        // SAFETY: the profile data and the venue are live; the out-pointer is valid for writes.
        let handle =
            Handle::create(|t, out| unsafe { sys::dxfg_v4_Schedule_getInstanceWithVenue(t, p, venue.as_ptr(), out) })?;
        Ok(Schedule { handle })
    }

    /// The schedule of a definition, such as `NewYorkETH()` or `name=US ETH;tz=America/New_York;...`
    /// (`getInstance(String)`).
    ///
    /// Returns an error if the definition is not valid.
    #[doc(alias = "getInstance")]
    pub fn get_instance_by_definition(definition: &str) -> Result<Self> {
        let definition = c_string(definition, "the definition")?;
        // SAFETY: the definition is live; the out-pointer is valid for writes.
        let handle = Handle::create(|t, out| unsafe {
            sys::dxfg_v4_Schedule_getInstanceByDefinition(t, definition.as_ptr(), out)
        })?;
        Ok(Schedule { handle })
    }

    /// The trading venues of a profile, the names of the schedules in its trading hours (`getTradingVenues`).
    #[doc(alias = "getTradingVenues")]
    pub fn get_trading_venues(profile: &InstrumentProfile) -> Result<Vec<String>> {
        let mut profile = profile.to_sys()?;
        let p = &raw mut profile.data;
        let mut list = ptr::null_mut();
        // SAFETY: the profile data is live; the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_Schedule_getTradingVenues(t, p, &mut list) })?;
        if list.is_null() {
            return Ok(Vec::new());
        }
        // SAFETY: the SDK filled the list, which is released once after the copy.
        let venues = unsafe {
            let list = &*list;
            match list.size {
                size if size <= 0 => Vec::new(),
                size => std::slice::from_raw_parts(list.elements, size as usize)
                    .iter()
                    .map(|&venue| isolate::string(venue).into_owned())
                    .collect(),
            }
        };
        // SAFETY: as above; nothing uses it afterwards.
        let _ = call(|t| unsafe { sys::dxfg_v4_string_list_release(t, list) });
        Ok(venues)
    }

    /// Downloads the defaults from a URL, once or every period: `<url>[,<period>]`, such as
    /// `https://example.com/schedule.zip,1d`; `auto` for the dxFeed URL every day, an empty string to stop
    /// (`downloadDefaults`).
    ///
    /// The first download is done before the function returns; a failed download is logged by the SDK and is not an
    /// error. Returns an error for an invalid configuration.
    #[doc(alias = "downloadDefaults")]
    pub fn download_defaults(config: &str) -> Result<()> {
        let config = c_string(config, "the download configuration")?;
        // SAFETY: the configuration is live.
        call(|t| unsafe { sys::dxfg_v4_Schedule_downloadDefaults(t, config.as_ptr()) })
    }

    /// Replaces the defaults by the content of a defaults file, plain or zipped (`setDefaults`).
    ///
    /// The defaults are ignored if their `date` is older than that of the defaults in use. Returns an error if
    /// they are not valid.
    #[doc(alias = "setDefaults")]
    pub fn set_defaults(data: &[u8]) -> Result<()> {
        let size = i32::try_from(data.len())
            .map_err(|_| Error::InvalidArgument(format!("the defaults are too long: {} bytes", data.len())))?;
        // SAFETY: the SDK only reads `size` bytes of the data.
        call(|t| unsafe { sys::dxfg_v4_Schedule_setDefaults(t, data.as_ptr().cast(), size) })
    }

    /// The session that contains a time (`getSessionByTime`).
    ///
    /// Returns an error if the time is outside the supported range.
    #[doc(alias = "getSessionByTime")]
    pub fn get_session_by_time(&self, time: i64) -> Result<Session> {
        let s = self.handle.as_ptr();
        // SAFETY: the schedule is live; the out-pointer is valid for writes.
        Session::create(|t, out| unsafe { sys::dxfg_v4_Schedule_getSessionByTime(t, s, time, out) })
    }

    /// The day that contains a time (`getDayByTime`).
    ///
    /// Returns an error if the time is outside the supported range.
    #[doc(alias = "getDayByTime")]
    pub fn get_day_by_time(&self, time: i64) -> Result<Day> {
        let s = self.handle.as_ptr();
        // SAFETY: the schedule is live; the out-pointer is valid for writes.
        Day::create(|t, out| unsafe { sys::dxfg_v4_Schedule_getDayByTime(t, s, time, out) })
    }

    /// The day of a day id, the days since 1970-01-01 (`getDayById`).
    #[doc(alias = "getDayById")]
    pub fn get_day_by_id(&self, day_id: i32) -> Result<Day> {
        let s = self.handle.as_ptr();
        // SAFETY: the schedule is live; the out-pointer is valid for writes.
        Day::create(|t, out| unsafe { sys::dxfg_v4_Schedule_getDayById(t, s, day_id, out) })
    }

    /// The day of a `yyyymmdd` number, such as `20261218` (`getDayByYearMonthDay`).
    #[doc(alias = "getDayByYearMonthDay")]
    pub fn get_day_by_year_month_day(&self, year_month_day: i32) -> Result<Day> {
        let s = self.handle.as_ptr();
        // SAFETY: the schedule is live; the out-pointer is valid for writes.
        Day::create(|t, out| unsafe { sys::dxfg_v4_Schedule_getDayByYearMonthDay(t, s, year_month_day, out) })
    }

    /// The session accepted by a filter that is nearest to a time: the one that contains it, else the nearest one
    /// within a year (`getNearestSessionByTime`).
    ///
    /// Returns an error if there is no such session.
    #[doc(alias = "getNearestSessionByTime")]
    pub fn get_nearest_session_by_time(&self, time: i64, filter: SessionFilter) -> Result<Session> {
        let filter = filter.handle()?;
        let (s, f) = (self.handle.as_ptr(), filter.as_ptr());
        // SAFETY: the schedule and the filter are live; the out-pointer is valid for writes.
        Session::create(|t, out| unsafe { sys::dxfg_v4_Schedule_getNearestSessionByTime(t, s, time, f, out) })
    }

    /// Like [`get_nearest_session_by_time`](Self::get_nearest_session_by_time), but `None` if there is no such
    /// session (`findNearestSessionByTime`).
    #[doc(alias = "findNearestSessionByTime")]
    pub fn find_nearest_session_by_time(&self, time: i64, filter: SessionFilter) -> Result<Option<Session>> {
        let filter = filter.handle()?;
        let (s, f) = (self.handle.as_ptr(), filter.as_ptr());
        // SAFETY: the schedule and the filter are live; the out-pointer is valid for writes.
        Session::create_optional(|t, out| unsafe { sys::dxfg_v4_Schedule_findNearestSessionByTime(t, s, time, f, out) })
    }

    string_getter!(
        /// The name of the schedule, such as `US ETH` (`getName`).
        #[doc(alias = "getName")]
        name: dxfg_v4_Schedule_getName
    );

    string_getter!(
        /// The display name of the time zone of the schedule, such as `Eastern Standard Time` (`getTimeZone`).
        #[doc(alias = "getTimeZone")]
        time_zone: dxfg_v4_Schedule_getTimeZone
    );

    string_getter!(
        /// The identifier of the time zone of the schedule, such as `America/New_York` (`getTimeZone().getID()`).
        time_zone_id: dxfg_v4_Schedule_getTimeZone_getID
    );

    string_getter!(
        /// The definition of the schedule, the text between the parentheses of the trading hours
        /// (`getDefinition`).
        #[doc(alias = "getDefinition")]
        definition: dxfg_v4_Schedule_getDefinition
    );
}

/// A day of a schedule: from its start to its end, with its sessions covering it without gaps (`Day`).
///
/// Days are equal when their schedules are equal and they have the same day id; the text is Java's `toString`.
pub struct Day {
    handle: Handle<sys::dxfg_v4_day_t>,
}

impl fmt::Debug for Day {
    /// The text of Java's `toString`, which names the type.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

java_object!(Day, dxfg_v4_Day_toString, dxfg_v4_Day_equals, dxfg_v4_Day_hashCode);

impl Day {
    fn create(f: impl FnOnce(isolate::Thread, *mut *mut sys::dxfg_v4_day_t) -> i32) -> Result<Self> {
        Ok(Day {
            handle: Handle::create(f)?,
        })
    }

    fn create_optional(f: impl FnOnce(isolate::Thread, *mut *mut sys::dxfg_v4_day_t) -> i32) -> Result<Option<Self>> {
        Ok(Handle::create_optional(f)?.map(|handle| Day { handle }))
    }

    /// The schedule of the day (`getSchedule`).
    #[doc(alias = "getSchedule")]
    pub fn schedule(&self) -> Result<Schedule> {
        let d = self.handle.as_ptr();
        // SAFETY: the day is live; the out-pointer is valid for writes.
        let handle = Handle::create(|t, out| unsafe { sys::dxfg_v4_Day_getSchedule(t, d, out) })?;
        Ok(Schedule { handle })
    }

    getter!(
        /// The day id, the days since 1970-01-01 (`getDayId`).
        #[doc(alias = "getDayId")]
        day_id: dxfg_v4_Day_getDayId -> i32 = 0
    );

    getter!(
        /// The `yyyymmdd` number of the day, such as `20261218` (`getYearMonthDay`).
        #[doc(alias = "getYearMonthDay")]
        year_month_day: dxfg_v4_Day_getYearMonthDay -> i32 = 0
    );

    getter!(
        /// The year (`getYear`).
        #[doc(alias = "getYear")]
        year: dxfg_v4_Day_getYear -> i32 = 0
    );

    getter!(
        /// The month of the year, 1 for January (`getMonthOfYear`).
        #[doc(alias = "getMonthOfYear")]
        month_of_year: dxfg_v4_Day_getMonthOfYear -> i32 = 0
    );

    getter!(
        /// The day of the month, from 1 (`getDayOfMonth`).
        #[doc(alias = "getDayOfMonth")]
        day_of_month: dxfg_v4_Day_getDayOfMonth -> i32 = 0
    );

    getter!(
        /// The day of the week, 1 for Monday to 7 for Sunday (`getDayOfWeek`).
        #[doc(alias = "getDayOfWeek")]
        day_of_week: dxfg_v4_Day_getDayOfWeek -> i32 = 0
    );

    getter!(
        /// Whether the day is a holiday (`isHoliday`).
        #[doc(alias = "isHoliday")]
        is_holiday: dxfg_v4_Day_isHoliday -> bool = 0, |v| v != 0
    );

    getter!(
        /// Whether the day is a short day, when trading ends earlier (`isShortDay`).
        #[doc(alias = "isShortDay")]
        is_short_day: dxfg_v4_Day_isShortDay -> bool = 0, |v| v != 0
    );

    getter!(
        /// Whether trading is allowed on the day (`isTrading`).
        #[doc(alias = "isTrading")]
        is_trading: dxfg_v4_Day_isTrading -> bool = 0, |v| v != 0
    );

    getter!(
        /// The start of the day, inclusive (`getStartTime`).
        #[doc(alias = "getStartTime")]
        start_time: dxfg_v4_Day_getStartTime -> i64 = 0
    );

    getter!(
        /// The end of the day, exclusive (`getEndTime`).
        #[doc(alias = "getEndTime")]
        end_time: dxfg_v4_Day_getEndTime -> i64 = 0
    );

    getter!(
        /// The time when the daily data is reset, such as the volume of the day (`getResetTime`).
        #[doc(alias = "getResetTime")]
        reset_time: dxfg_v4_Day_getResetTime -> i64 = 0
    );

    /// Whether the day contains a time (`containsTime`).
    #[doc(alias = "containsTime")]
    pub fn contains_time(&self, time: i64) -> Result<bool> {
        let mut contains = 0;
        let d = self.handle.as_ptr();
        // SAFETY: the day is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_Day_containsTime(t, d, time, &mut contains) })?;
        Ok(contains != 0)
    }

    /// The sessions of the day, in the order of time (`getSessions`).
    #[doc(alias = "getSessions")]
    pub fn sessions(&self) -> Result<Vec<Session>> {
        let mut list = ptr::null_mut();
        let d = self.handle.as_ptr();
        // SAFETY: the day is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_Day_getSessions(t, d, &mut list) })?;
        if list.is_null() {
            return Ok(Vec::new());
        }
        // SAFETY: the SDK filled the list; each session is ours, the list itself is released once below.
        let sessions = unsafe {
            let list = &*list;
            match list.size {
                size if size <= 0 => Vec::new(),
                size => std::slice::from_raw_parts(list.elements, size as usize)
                    .iter()
                    .filter_map(|&session| Handle::from_raw(session))
                    .map(|handle| Session { handle })
                    .collect(),
            }
        };
        // SAFETY: as above; the sessions are kept.
        let _ = call(|t| unsafe { sys::dxfg_v4_session_list_wrapper_release(t, list) });
        Ok(sessions)
    }

    /// The session that contains a time (`getSessionByTime`).
    ///
    /// Returns an error if the day does not contain the time.
    #[doc(alias = "getSessionByTime")]
    pub fn get_session_by_time(&self, time: i64) -> Result<Session> {
        let d = self.handle.as_ptr();
        // SAFETY: the day is live; the out-pointer is valid for writes.
        Session::create(|t, out| unsafe { sys::dxfg_v4_Day_getSessionByTime(t, d, time, out) })
    }

    /// The first session of the day accepted by a filter (`getFirstSession`).
    ///
    /// Returns an error if there is no such session.
    #[doc(alias = "getFirstSession")]
    pub fn get_first_session(&self, filter: SessionFilter) -> Result<Session> {
        self.session(filter, sys::dxfg_v4_Day_getFirstSession)
    }

    /// The last session of the day accepted by a filter (`getLastSession`).
    ///
    /// Returns an error if there is no such session.
    #[doc(alias = "getLastSession")]
    pub fn get_last_session(&self, filter: SessionFilter) -> Result<Session> {
        self.session(filter, sys::dxfg_v4_Day_getLastSession)
    }

    /// The first session of the day accepted by a filter, or `None` (`findFirstSession`).
    #[doc(alias = "findFirstSession")]
    pub fn find_first_session(&self, filter: SessionFilter) -> Result<Option<Session>> {
        self.find_session(filter, sys::dxfg_v4_Day_findFirstSession)
    }

    /// The last session of the day accepted by a filter, or `None` (`findLastSession`).
    #[doc(alias = "findLastSession")]
    pub fn find_last_session(&self, filter: SessionFilter) -> Result<Option<Session>> {
        self.find_session(filter, sys::dxfg_v4_Day_findLastSession)
    }

    /// The previous day accepted by a filter, within a year (`getPrevDay`).
    ///
    /// Returns an error if there is no such day.
    #[doc(alias = "getPrevDay")]
    pub fn get_prev_day(&self, filter: DayFilter) -> Result<Day> {
        self.day(filter, sys::dxfg_v4_Day_getPrevDay)
    }

    /// The next day accepted by a filter, within a year (`getNextDay`).
    ///
    /// Returns an error if there is no such day.
    #[doc(alias = "getNextDay")]
    pub fn get_next_day(&self, filter: DayFilter) -> Result<Day> {
        self.day(filter, sys::dxfg_v4_Day_getNextDay)
    }

    /// The previous day accepted by a filter within a year, or `None` (`findPrevDay`).
    #[doc(alias = "findPrevDay")]
    pub fn find_prev_day(&self, filter: DayFilter) -> Result<Option<Day>> {
        self.find_day(filter, sys::dxfg_v4_Day_findPrevDay)
    }

    /// The next day accepted by a filter within a year, or `None` (`findNextDay`).
    #[doc(alias = "findNextDay")]
    pub fn find_next_day(&self, filter: DayFilter) -> Result<Option<Day>> {
        self.find_day(filter, sys::dxfg_v4_Day_findNextDay)
    }

    fn session(&self, filter: SessionFilter, f: DaySessionFn) -> Result<Session> {
        let filter = filter.handle()?;
        let (d, sf) = (self.handle.as_ptr(), filter.as_ptr());
        // SAFETY: the day and the filter are live; the out-pointer is valid for writes.
        Session::create(|t, out| unsafe { f(t, d, sf, out) })
    }

    fn find_session(&self, filter: SessionFilter, f: DaySessionFn) -> Result<Option<Session>> {
        let filter = filter.handle()?;
        let (d, sf) = (self.handle.as_ptr(), filter.as_ptr());
        // SAFETY: the day and the filter are live; the out-pointer is valid for writes.
        Session::create_optional(|t, out| unsafe { f(t, d, sf, out) })
    }

    fn day(&self, filter: DayFilter, f: DayDayFn) -> Result<Day> {
        let filter = filter.handle()?;
        let (d, df) = (self.handle.as_ptr(), filter.as_ptr());
        // SAFETY: the day and the filter are live; the out-pointer is valid for writes.
        Day::create(|t, out| unsafe { f(t, d, df, out) })
    }

    fn find_day(&self, filter: DayFilter, f: DayDayFn) -> Result<Option<Day>> {
        let filter = filter.handle()?;
        let (d, df) = (self.handle.as_ptr(), filter.as_ptr());
        // SAFETY: the day and the filter are live; the out-pointer is valid for writes.
        Day::create_optional(|t, out| unsafe { f(t, d, df, out) })
    }
}

type DaySessionFn = unsafe extern "C" fn(
    isolate::Thread,
    *mut sys::dxfg_v4_day_t,
    *mut sys::dxfg_v4_session_filter_t,
    *mut *mut sys::dxfg_v4_session_t,
) -> i32;

type DayDayFn = unsafe extern "C" fn(
    isolate::Thread,
    *mut sys::dxfg_v4_day_t,
    *mut sys::dxfg_v4_day_filter_t,
    *mut *mut sys::dxfg_v4_day_t,
) -> i32;

type SessionSessionFn = unsafe extern "C" fn(
    isolate::Thread,
    *mut sys::dxfg_v4_session_t,
    *mut sys::dxfg_v4_session_filter_t,
    *mut *mut sys::dxfg_v4_session_t,
) -> i32;

/// A session of a day: a continuous period of one [`SessionType`] (`Session`).
///
/// Sessions are equal when their days are equal and they have the same type, start and end; the text is Java's
/// `toString`.
pub struct Session {
    handle: Handle<sys::dxfg_v4_session_t>,
}

impl fmt::Debug for Session {
    /// The text of Java's `toString`, which names the type.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

java_object!(
    Session,
    dxfg_v4_Session_toString,
    dxfg_v4_Session_equals,
    dxfg_v4_Session_hashCode
);

impl Session {
    fn create(f: impl FnOnce(isolate::Thread, *mut *mut sys::dxfg_v4_session_t) -> i32) -> Result<Self> {
        Ok(Session {
            handle: Handle::create(f)?,
        })
    }

    fn create_optional(
        f: impl FnOnce(isolate::Thread, *mut *mut sys::dxfg_v4_session_t) -> i32,
    ) -> Result<Option<Self>> {
        Ok(Handle::create_optional(f)?.map(|handle| Session { handle }))
    }

    /// The day of the session (`getDay`).
    #[doc(alias = "getDay")]
    pub fn day(&self) -> Result<Day> {
        let s = self.handle.as_ptr();
        // SAFETY: the session is live; the out-pointer is valid for writes.
        Day::create(|t, out| unsafe { sys::dxfg_v4_Session_getDay(t, s, out) })
    }

    /// The type of the session (`getType`).
    #[doc(alias = "getType")]
    pub fn session_type(&self) -> Result<SessionType> {
        let mut session_type = sys::DXFG_SESSION_TYPE_NO_TRADING;
        let s = self.handle.as_ptr();
        // SAFETY: the session is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_Session_getType(t, s, &mut session_type) })?;
        SessionType::from_sys(session_type)
    }

    getter!(
        /// Whether trading is allowed in the session (`isTrading`).
        #[doc(alias = "isTrading")]
        is_trading: dxfg_v4_Session_isTrading -> bool = 0, |v| v != 0
    );

    getter!(
        /// Whether the session is empty: its start equals its end (`isEmpty`).
        #[doc(alias = "isEmpty")]
        is_empty: dxfg_v4_Session_isEmpty -> bool = 0, |v| v != 0
    );

    getter!(
        /// The start of the session, inclusive (`getStartTime`).
        #[doc(alias = "getStartTime")]
        start_time: dxfg_v4_Session_getStartTime -> i64 = 0
    );

    getter!(
        /// The end of the session, exclusive (`getEndTime`).
        #[doc(alias = "getEndTime")]
        end_time: dxfg_v4_Session_getEndTime -> i64 = 0
    );

    /// Whether the session contains a time (`containsTime`).
    #[doc(alias = "containsTime")]
    pub fn contains_time(&self, time: i64) -> Result<bool> {
        let mut contains = 0;
        let s = self.handle.as_ptr();
        // SAFETY: the session is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_Session_containsTime(t, s, time, &mut contains) })?;
        Ok(contains != 0)
    }

    /// The previous session accepted by a filter, within a year (`getPrevSession`).
    ///
    /// Returns an error if there is no such session.
    #[doc(alias = "getPrevSession")]
    pub fn get_prev_session(&self, filter: SessionFilter) -> Result<Session> {
        self.session(filter, sys::dxfg_v4_Session_getPrevSession)
    }

    /// The next session accepted by a filter, within a year (`getNextSession`).
    ///
    /// Returns an error if there is no such session.
    #[doc(alias = "getNextSession")]
    pub fn get_next_session(&self, filter: SessionFilter) -> Result<Session> {
        self.session(filter, sys::dxfg_v4_Session_getNextSession)
    }

    /// The previous session accepted by a filter within a year, or `None` (`findPrevSession`).
    #[doc(alias = "findPrevSession")]
    pub fn find_prev_session(&self, filter: SessionFilter) -> Result<Option<Session>> {
        self.find_session(filter, sys::dxfg_v4_Session_findPrevSession)
    }

    /// The next session accepted by a filter within a year, or `None` (`findNextSession`).
    #[doc(alias = "findNextSession")]
    pub fn find_next_session(&self, filter: SessionFilter) -> Result<Option<Session>> {
        self.find_session(filter, sys::dxfg_v4_Session_findNextSession)
    }

    fn session(&self, filter: SessionFilter, f: SessionSessionFn) -> Result<Session> {
        let filter = filter.handle()?;
        let (s, sf) = (self.handle.as_ptr(), filter.as_ptr());
        // SAFETY: the session and the filter are live; the out-pointer is valid for writes.
        Session::create(|t, out| unsafe { f(t, s, sf, out) })
    }

    fn find_session(&self, filter: SessionFilter, f: SessionSessionFn) -> Result<Option<Session>> {
        let filter = filter.handle()?;
        let (s, sf) = (self.handle.as_ptr(), filter.as_ptr());
        // SAFETY: the session and the filter are live; the out-pointer is valid for writes.
        Session::create_optional(|t, out| unsafe { f(t, s, sf, out) })
    }
}
