//! Time periods, time zones and time formats of a zone, read and written by Java; no network needed.

use std::time::Duration;

use dxfeed_graal::{Error, TimeFormat, TimePeriod, TimeZone};

const HOUR: i32 = 3_600_000;

#[test]
fn periods_are_read_as_in_java() {
    let millis = |text| TimePeriod::value_of(text).unwrap().get_time();
    assert_eq!(millis("1s"), 1000);
    assert_eq!(millis("PT1H30M"), 5_400_000);
    assert_eq!(millis("1.5"), 1500);
    assert_eq!(millis("P1D"), 86_400_000);
    assert_eq!(millis("1d2h"), 93_600_000);
    assert_eq!(TimePeriod::value_of("inf").unwrap(), TimePeriod::UNLIMITED);
    assert_eq!(TimePeriod::value_of("0s").unwrap(), TimePeriod::ZERO);
    assert_eq!("2m".parse::<TimePeriod>().unwrap().get_time(), 120_000);
    for text in ["x", "", "1q", "PT"] {
        assert!(matches!(TimePeriod::value_of(text), Err(Error::Java(_))), "{text:?}");
    }
    assert_eq!(
        TimePeriod::value_of_or("", TimePeriod::UNLIMITED).unwrap(),
        TimePeriod::UNLIMITED
    );
    assert_eq!(
        TimePeriod::value_of_or("1s", TimePeriod::UNLIMITED).unwrap().get_time(),
        1000
    );
    assert!(matches!(TimePeriod::value_of("1\0s"), Err(Error::InvalidArgument(_))));
}

#[test]
fn periods_read_back_their_text() {
    // Not UNLIMITED: its days overflow the `int` that Java reads them into.
    for millis in [0, 1, 999, 1000, 1500, 5_400_000, 86_400_000, 90_061_001] {
        let period = TimePeriod::value_of_millis(millis);
        assert_eq!(TimePeriod::value_of(&period.to_string()).unwrap(), period, "{period}");
    }
    let duration = Duration::from_millis(5_400_000);
    assert_eq!(
        Duration::try_from(TimePeriod::try_from(duration).unwrap()).unwrap(),
        duration
    );
}

#[test]
fn zones_give_offsets_and_names() {
    let winter = TimeFormat::GMT.parse("20260115-120000").unwrap();
    let summer = TimeFormat::GMT.parse("20260715-120000").unwrap();
    let zone = TimeZone::get_time_zone("America/New_York").unwrap();
    assert_eq!(zone.get_id(), "America/New_York");
    assert_eq!(zone.get_offset(winter).unwrap(), -5 * HOUR);
    assert_eq!(zone.get_offset(summer).unwrap(), -4 * HOUR);
    assert_eq!(zone.get_raw_offset().unwrap(), -5 * HOUR);
    assert_eq!(zone.get_dst_savings().unwrap(), HOUR);
    assert!(zone.use_daylight_time().unwrap());
    assert!(zone.observes_daylight_time().unwrap());
    assert!(!zone.in_daylight_time(winter).unwrap());
    assert!(zone.in_daylight_time(summer).unwrap());
    // 2026-01-15 is a Thursday, day 5 of Java's week.
    assert_eq!(
        zone.get_offset_by_calendar_fields(1, 2026, 0, 15, 5, 0).unwrap(),
        -5 * HOUR
    );
    assert!(matches!(
        zone.get_offset_by_calendar_fields(1, 2026, 13, 15, 5, 0),
        Err(Error::Java(_))
    ));
    // The names depend on the locale data of the SDK, which has generic names only, such as `New York (+0)`.
    assert!(!zone.get_display_name().unwrap().is_empty());
    let short = zone.get_display_name_with_style(false, TimeZone::SHORT).unwrap();
    let daylight = zone.get_display_name_with_style(true, TimeZone::SHORT).unwrap();
    assert!(
        !short.is_empty() && !daylight.is_empty() && short != daylight,
        "{short} {daylight}"
    );
    assert!(matches!(
        zone.get_display_name_with_style(false, 2),
        Err(Error::Java(_))
    ));

    let alias = TimeZone::get_time_zone("US/Eastern").unwrap();
    assert_ne!(alias, zone);
    assert!(alias.has_same_rules(&zone).unwrap());
    assert_eq!(zone.clone(), TimeZone::get_time_zone("America/New_York").unwrap());

    let fixed = TimeZone::get_time_zone("GMT+05:30").unwrap();
    assert_eq!(fixed.get_raw_offset().unwrap(), 5 * HOUR + HOUR / 2);
    assert!(!fixed.use_daylight_time().unwrap());
    assert_eq!(fixed.get_display_name().unwrap(), "GMT+05:30");
    assert_eq!(TimeZone::get_time_zone("Nowhere/Zone").unwrap().get_id(), "GMT");
    assert!(!TimeZone::get_default().unwrap().get_id().is_empty());
    assert!(matches!(
        TimeZone::get_time_zone("a\0b"),
        Err(Error::InvalidArgument(_))
    ));
}

#[test]
fn formats_use_their_zone() {
    let utc = TimeFormat::get_instance(&TimeZone::get_time_zone("UTC").unwrap());
    // Java writes the zero time as `0`.
    assert_eq!(utc.format(0).unwrap(), "0");
    assert_eq!(utc.format(1000).unwrap(), "19700101-000001");
    assert_eq!(utc.with_millis().format(1).unwrap(), "19700101-000000.001");
    assert_eq!(utc.get_time_zone().unwrap().get_id(), "UTC");

    let zone = TimeZone::get_time_zone("America/New_York").unwrap();
    let new_york = TimeFormat::get_instance(&zone);
    assert_eq!(new_york, TimeFormat::get_instance(&zone.clone()));
    assert_ne!(new_york, utc);
    assert_eq!(new_york.format(1000).unwrap(), "19691231-190001");
    assert_eq!(new_york.with_time_zone().format(1000).unwrap(), "19691231-190001-0500");
    assert_eq!(new_york.parse("19700101-000000").unwrap(), 5 * i64::from(HOUR));
    assert_eq!(new_york.get_time_zone().unwrap(), zone);
    assert_eq!(new_york.with_millis().as_full_iso().get_time_zone().unwrap(), zone);

    assert_eq!(TimeFormat::GMT.get_time_zone().unwrap().get_id(), "GMT");
    assert_eq!(
        TimeFormat::DEFAULT.get_time_zone().unwrap(),
        TimeZone::get_default().unwrap()
    );
}
