//! Trading schedules from definitions, defaults and profiles, at fixed times; no network needed.
//!
//! The defaults set here replace those of the SDK for this test process, so the other tests use definitions that
//! need no defaults. Java logs an error for each cached schedule whose venue the new defaults drop.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::thread;

use dxfeed_graal::ipf::InstrumentProfile;
use dxfeed_graal::schedule::{CustomDayFilter, CustomSessionFilter, DayFilter, Schedule, SessionFilter, SessionType};
use dxfeed_graal::{Error, ErrorManager};

/// The US extended trading hours without holidays: pre-market 04:00, regular 09:30, after-market 16:00 to 20:00.
const DEFINITION: &str = "Test(name=Test;tz=America/New_York;td=12345;de=+0000;0=p04000930r09301600a16002000)";

const HOUR: i64 = 3_600_000;
/// 2026-10-28 00:00 UTC, a Wednesday; New York is at UTC-4 until November 1.
const OCTOBER_28_2026: i64 = 20754 * 24 * HOUR;

fn schedule() -> Schedule {
    Schedule::get_instance_by_definition(DEFINITION).unwrap()
}

fn minutes(hours: i64, minutes: i64) -> i64 {
    hours * HOUR + minutes * 60_000
}

#[test]
fn a_trading_day_and_its_sessions() {
    let schedule = schedule();
    assert_eq!(schedule.get_name().unwrap(), "Test");
    assert_eq!(schedule.get_time_zone().unwrap().get_id(), "America/New_York");
    assert_eq!(schedule.get_definition().unwrap(), DEFINITION);
    assert_eq!(schedule.to_string(), format!("Schedule{{{DEFINITION}}}"));

    let day = schedule.get_day_by_year_month_day(20261028).unwrap();
    assert_eq!(day.get_day_id().unwrap(), 20754);
    assert_eq!(day.get_year_month_day().unwrap(), 20261028);
    assert_eq!(
        (
            day.get_year().unwrap(),
            day.get_month_of_year().unwrap(),
            day.get_day_of_month().unwrap(),
            day.get_day_of_week().unwrap()
        ),
        (2026, 10, 28, 3)
    );
    assert!(day.is_trading().unwrap());
    assert!(!day.is_holiday().unwrap());
    assert!(!day.is_short_day().unwrap());
    let midnight = OCTOBER_28_2026 + 4 * HOUR;
    assert_eq!(day.get_start_time().unwrap(), midnight);
    assert_eq!(day.get_end_time().unwrap(), midnight + 24 * HOUR);
    assert_eq!(day.get_reset_time().unwrap(), midnight);
    assert!(day.contains_time(midnight).unwrap());
    assert!(!day.contains_time(midnight + 24 * HOUR).unwrap());
    assert!(day.to_string().starts_with("Day(20261028, true, "), "{day}");

    let sessions: Vec<_> = day
        .get_sessions()
        .unwrap()
        .iter()
        .map(|s| {
            (
                s.get_type().unwrap(),
                s.get_start_time().unwrap() - midnight,
                s.get_end_time().unwrap() - midnight,
            )
        })
        .collect();
    assert_eq!(
        sessions,
        [
            (SessionType::NoTrading, 0, minutes(4, 0)),
            (SessionType::PreMarket, minutes(4, 0), minutes(9, 30)),
            (SessionType::Regular, minutes(9, 30), minutes(16, 0)),
            (SessionType::AfterMarket, minutes(16, 0), minutes(20, 0)),
            (SessionType::NoTrading, minutes(20, 0), minutes(24, 0)),
        ]
    );

    let regular = day.get_session_by_time(midnight + 12 * HOUR).unwrap();
    assert_eq!(regular.get_type().unwrap(), SessionType::Regular);
    assert!(regular.is_trading().unwrap());
    assert!(!regular.is_empty().unwrap());
    assert!(regular.contains_time(midnight + minutes(9, 30)).unwrap());
    assert!(
        regular.to_string().starts_with("Session(20261028, REGULAR, true, "),
        "{regular}"
    );
    assert_eq!(regular.get_day().unwrap(), day);
    assert_eq!(day.get_first_session(SessionFilter::Regular).unwrap(), regular);
    let last = day.get_last_session(SessionFilter::Trading).unwrap();
    assert_eq!(last.get_type().unwrap(), SessionType::AfterMarket);
}

#[test]
fn days_and_sessions_are_found_across_the_weekend() {
    let schedule = schedule();
    let saturday = schedule.get_day_by_year_month_day(20261031).unwrap();
    assert!(!saturday.is_trading().unwrap());
    assert_eq!(saturday.get_sessions().unwrap().len(), 1);
    assert!(saturday.find_first_session(SessionFilter::Trading).unwrap().is_none());
    match saturday.get_first_session(SessionFilter::Trading) {
        Err(Error::Java(e)) => assert_eq!(e.get_class_name(), "java.util.NoSuchElementException"),
        other => panic!("expected NoSuchElementException, got {other:?}"),
    }

    assert_eq!(
        saturday
            .get_next_day(DayFilter::Trading)
            .unwrap()
            .get_year_month_day()
            .unwrap(),
        20261102
    );
    let friday = saturday.find_prev_day(DayFilter::Trading).unwrap().unwrap();
    assert_eq!(friday.get_year_month_day().unwrap(), 20261030);
    assert_eq!(saturday.get_prev_day(DayFilter::Friday).unwrap(), friday);
    let sunday = saturday.find_next_day(DayFilter::WeekEnd).unwrap().unwrap();
    assert_eq!(sunday.get_year_month_day().unwrap(), 20261101);
    assert!(saturday.find_next_day(DayFilter::Holiday).unwrap().is_none());

    // Friday's after-market session is followed by Monday's pre-market one, at 04:00 New York time, UTC-5 by then.
    let friday_evening = OCTOBER_28_2026 + 2 * 24 * HOUR + 22 * HOUR;
    let after_market = schedule.get_session_by_time(friday_evening).unwrap();
    assert_eq!(after_market.get_type().unwrap(), SessionType::AfterMarket);
    let next = after_market.get_next_session(SessionFilter::Trading).unwrap();
    assert_eq!(next.get_type().unwrap(), SessionType::PreMarket);
    assert_eq!(
        next.get_start_time().unwrap(),
        OCTOBER_28_2026 + 5 * 24 * HOUR + 9 * HOUR
    );
    assert_eq!(
        next.find_prev_session(SessionFilter::AfterMarket).unwrap().unwrap(),
        after_market
    );
    assert_eq!(
        next.get_prev_session(SessionFilter::Any).unwrap().get_type().unwrap(),
        SessionType::NoTrading
    );

    // Saturday noon is nearer to Friday's after-market session than to Monday's pre-market one.
    let saturday_noon = saturday.get_start_time().unwrap() + 12 * HOUR;
    let nearest = schedule
        .get_nearest_session_by_time(saturday_noon, SessionFilter::Trading)
        .unwrap();
    assert_eq!(nearest, after_market);
    let found = schedule
        .find_nearest_session_by_time(saturday_noon, SessionFilter::Trading)
        .unwrap();
    assert_eq!(found, Some(nearest));
}

#[test]
fn each_filter_finds_its_days_and_sessions() {
    let schedule = schedule();
    // Sunday, November 1, 2026: the next Monday to Saturday follow it in order.
    let sunday = schedule.get_day_by_year_month_day(20261101).unwrap();
    let weekdays = [
        (DayFilter::Monday, 1),
        (DayFilter::Tuesday, 2),
        (DayFilter::Wednesday, 3),
        (DayFilter::Thursday, 4),
        (DayFilter::Friday, 5),
        (DayFilter::Saturday, 6),
    ];
    for (filter, day_of_week) in weekdays {
        let day = sunday.get_next_day(filter).unwrap();
        assert_eq!(day.get_day_of_week().unwrap(), day_of_week, "{filter:?}");
        assert_eq!(day.get_year_month_day().unwrap(), 20261101 + day_of_week, "{filter:?}");
    }
    let saturday = schedule.get_day_by_year_month_day(20261031).unwrap();
    assert_eq!(saturday.get_next_day(DayFilter::Sunday).unwrap(), sunday);
    assert_eq!(saturday.get_next_day(DayFilter::Any).unwrap(), sunday);
    assert_eq!(saturday.get_next_day(DayFilter::NonTrading).unwrap(), sunday);
    assert_eq!(
        saturday
            .get_prev_day(DayFilter::WeekDay)
            .unwrap()
            .get_day_of_week()
            .unwrap(),
        5
    );
    assert!(saturday.find_next_day(DayFilter::ShortDay).unwrap().is_none());

    let day = schedule.get_day_by_year_month_day(20261028).unwrap();
    let sessions = [
        (SessionFilter::NoTrading, SessionType::NoTrading),
        (SessionFilter::NonTrading, SessionType::NoTrading),
        (SessionFilter::PreMarket, SessionType::PreMarket),
        (SessionFilter::Trading, SessionType::PreMarket),
        (SessionFilter::Regular, SessionType::Regular),
        (SessionFilter::AfterMarket, SessionType::AfterMarket),
    ];
    for (filter, session_type) in sessions {
        let session = day.get_first_session(filter).unwrap();
        assert_eq!(session.get_type().unwrap(), session_type, "{filter:?}");
    }
}

#[test]
fn equal_days_are_equal_and_hash_alike() {
    let schedule = schedule();
    let by_id = schedule.get_day_by_id(20754).unwrap();
    let by_time = schedule.get_day_by_time(OCTOBER_28_2026 + 12 * HOUR).unwrap();
    assert_eq!(by_id, by_time);
    assert_ne!(by_id, schedule.get_day_by_id(20755).unwrap());
    assert_eq!(HashSet::from([by_id, by_time]).len(), 1);
}

#[test]
fn a_broken_definition_is_an_error() {
    match Schedule::get_instance_by_definition("Test(") {
        Err(Error::Java(e)) => assert_eq!(e.get_class_name(), "java.lang.IllegalArgumentException"),
        other => panic!("expected IllegalArgumentException, got {other:?}"),
    }
}

#[test]
fn the_defaults_give_the_venues_and_holidays_of_profiles() {
    let defaults = concat!(
        "date=20990101-000000+0000\n",
        "hd.RUSTTEST=20261028,20261225\n",
        "tv.RustVenue=name=Rust Venue;tz=America/New_York;hd=RUSTTEST;td=12345;de=+0000;0=09301600\n",
    );
    Schedule::set_defaults(defaults.as_bytes()).unwrap();

    let profile = InstrumentProfile {
        instrument_type: "STOCK".into(),
        symbol: "TEST".into(),
        trading_hours: "RustVenue()".into(),
        ..InstrumentProfile::new()
    };
    assert_eq!(Schedule::get_trading_venues(&profile).unwrap(), ["RustVenue"]);
    let schedule = Schedule::get_instance(&profile).unwrap();
    assert_eq!(schedule.get_name().unwrap(), "Rust Venue");
    let holiday = schedule.get_day_by_year_month_day(20261028).unwrap();
    assert!(holiday.is_holiday().unwrap());
    assert!(!holiday.is_trading().unwrap());
    let next_holiday = holiday.get_next_day(DayFilter::Holiday).unwrap();
    assert_eq!(next_holiday.get_year_month_day().unwrap(), 20261225);

    let by_venue = Schedule::get_instance_with_venue(&profile, "RustVenue").unwrap();
    assert_eq!(by_venue.get_day_by_id(20754).unwrap(), holiday);
    assert!(Schedule::get_instance_with_venue(&profile, "NoSuchVenue").is_err());
}

#[test]
fn custom_filters_have_the_conditions_of_java() {
    let schedule = schedule();
    let saturday = schedule.get_day_by_year_month_day(20261031).unwrap();
    let wednesdays = CustomDayFilter::new(CustomDayFilter::WEDNESDAY, None, None, Some(true)).unwrap();
    assert_eq!(wednesdays.to_string(), "DayFilter(8, null, null, true)");
    let wednesday = saturday.get_next_day(&wednesdays).unwrap();
    assert_eq!(wednesday.get_year_month_day().unwrap(), 20261104);
    assert!(wednesdays.accept(&wednesday).unwrap());
    assert!(!wednesdays.accept(&saturday).unwrap());
    let weekend = CustomDayFilter::new(CustomDayFilter::SATURDAY | CustomDayFilter::SUNDAY, None, None, None).unwrap();
    assert_eq!(
        saturday.get_prev_day(&weekend).unwrap().get_year_month_day().unwrap(),
        20261025
    );
    // A mask of 0 accepts any day of the week.
    let any = CustomDayFilter::new(0, None, Some(false), None).unwrap();
    assert_eq!(
        saturday.get_next_day(&any).unwrap().get_year_month_day().unwrap(),
        20261101
    );
    assert!(any.accept(&saturday).unwrap());
    let holidays = CustomDayFilter::new(0, Some(true), None, None).unwrap();
    assert!(saturday.find_next_day(&holidays).unwrap().is_none());
    assert!(!holidays.accept(&saturday).unwrap());

    let day = schedule.get_day_by_year_month_day(20261028).unwrap();
    let regular = CustomSessionFilter::new(Some(SessionType::Regular), Some(true)).unwrap();
    assert_eq!(regular.to_string(), "SessionFilter(REGULAR, true)");
    let session = day.get_first_session(&regular).unwrap();
    assert_eq!(session.get_type().unwrap(), SessionType::Regular);
    assert!(regular.accept(&session).unwrap());
    let none = CustomSessionFilter::new(Some(SessionType::Regular), Some(false)).unwrap();
    assert!(day.find_first_session(&none).unwrap().is_none());
    assert!(!none.accept(&session).unwrap());
    let trading = CustomSessionFilter::new(None, Some(true)).unwrap();
    assert_eq!(
        day.get_last_session(&trading).unwrap().get_type().unwrap(),
        SessionType::AfterMarket
    );
    let any = CustomSessionFilter::new(None, None).unwrap();
    assert_eq!(
        day.get_first_session(&any).unwrap().get_type().unwrap(),
        SessionType::NoTrading
    );
    assert_eq!(any.to_string(), "SessionFilter(null, null)");
    // A filter can be given by value too.
    let first = day
        .get_first_session(CustomSessionFilter::new(None, Some(true)).unwrap())
        .unwrap();
    assert_eq!(first.get_type().unwrap(), SessionType::PreMarket);
}

#[test]
fn a_predicate_is_asked_after_the_conditions() {
    let schedule = schedule();
    let saturday = schedule.get_day_by_year_month_day(20261031).unwrap();
    let asked = Arc::new(Mutex::new(Vec::new()));
    let days = Arc::clone(&asked);
    // The trading days of an odd day of the month.
    let odd = CustomDayFilter::with_predicate(0, None, None, Some(true), move |day| {
        let day_of_month = day.get_day_of_month().unwrap();
        days.lock().unwrap().push(day_of_month);
        day_of_month % 2 == 1
    })
    .unwrap();
    assert_eq!(
        saturday.get_next_day(&odd).unwrap().get_year_month_day().unwrap(),
        20261103
    );
    // Sunday, November 1, is not a trading day, so the predicate is not asked for it.
    assert_eq!(*asked.lock().unwrap(), [2, 3]);
    asked.lock().unwrap().clear();
    assert_eq!(
        saturday
            .find_prev_day(&odd)
            .unwrap()
            .unwrap()
            .get_year_month_day()
            .unwrap(),
        20261029
    );
    assert_eq!(*asked.lock().unwrap(), [30, 29]);
    assert!(!odd.accept(&saturday).unwrap());
    let tuesday = schedule.get_day_by_year_month_day(20261103).unwrap();
    assert!(odd.accept(&tuesday).unwrap());
    assert!(format!("{odd:?}").contains("predicate: true"), "{odd:?}");

    // The trading sessions that end at 18:00 of their day or later: the after-market ones only.
    let late = CustomSessionFilter::with_predicate(None, Some(true), |session| {
        let day_start = session.get_day().unwrap().get_start_time().unwrap();
        session.get_end_time().unwrap() >= day_start + 18 * HOUR
    })
    .unwrap();
    let day = schedule.get_day_by_year_month_day(20261028).unwrap();
    let after_market = day.get_first_session(&late).unwrap();
    assert_eq!(after_market.get_type().unwrap(), SessionType::AfterMarket);
    assert!(late.accept(&after_market).unwrap());
    let next = after_market.get_next_session(&late).unwrap();
    assert_eq!(next.get_day().unwrap().get_year_month_day().unwrap(), 20261029);
    assert_eq!(next.get_type().unwrap(), SessionType::AfterMarket);
    // Noon is in the regular session; the after-market one, from 16:00, is nearer than that of the day before.
    let noon = day.get_start_time().unwrap() + 12 * HOUR;
    assert_eq!(schedule.get_nearest_session_by_time(noon, &late).unwrap(), after_market);
}

#[test]
fn a_panicking_predicate_rejects_and_is_reported() {
    let schedule = schedule();
    let saturday = schedule.get_day_by_year_month_day(20261031).unwrap();
    let filter = CustomDayFilter::with_predicate(0, None, None, Some(true), |day| {
        assert_ne!(day.get_day_of_month().unwrap(), 2, "test panic in a day predicate");
        true
    })
    .unwrap();
    assert_eq!(
        saturday.get_next_day(&filter).unwrap().get_year_month_day().unwrap(),
        20261103
    );
    assert!(
        ErrorManager::get_errors()
            .iter()
            .any(|e| e.component == "Schedule" && e.message.contains("test panic in a day predicate")),
        "{:?}",
        ErrorManager::get_errors()
    );
}

#[test]
fn custom_filters_are_shared_by_threads() {
    let schedule = schedule();
    let filter = CustomDayFilter::with_predicate(CustomDayFilter::FRIDAY, None, None, None, |day| {
        day.get_day_of_month().is_ok_and(|d| d > 7)
    })
    .unwrap();
    thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| {
                let start = schedule.get_day_by_year_month_day(20261101).unwrap();
                // Friday, November 6, is the 6th, so the next accepted Friday is November 13.
                let friday = start.get_next_day(&filter).unwrap();
                assert_eq!(friday.get_year_month_day().unwrap(), 20261113);
            });
        }
    });
}
