//! The predefined schedules and filters of Java, with the defaults of the SDK; no network needed.

use dxfeed_graal::schedule::{DayFilter, Schedule, SessionFilter};

#[test]
fn the_utc_and_nyse_schedules_have_their_time_zones() {
    let utc = Schedule::utc().unwrap();
    assert_eq!(utc.get_time_zone().unwrap().get_id(), "UTC");
    assert_eq!(utc.to_string(), "Schedule{UTC(tz=UTC;td=12345;de=+0000;0=0000+0000)}");
    let nyse = Schedule::nyse().unwrap();
    assert_eq!(nyse.get_time_zone().unwrap().get_id(), "America/New_York");
    assert_eq!(nyse.get_name().unwrap(), "NYSE");
    assert_eq!(nyse.to_string(), "Schedule{XNYS()}");
}

#[test]
fn the_filters_display_the_java_text() {
    assert_eq!(DayFilter::Any.to_string(), "DayFilter(0, null, null, null)");
    assert_eq!(DayFilter::Trading.to_string(), "DayFilter(0, null, null, true)");
    assert_eq!(DayFilter::Monday.to_string(), "DayFilter(2, null, null, null)");
    assert_eq!(SessionFilter::Trading.to_string(), "SessionFilter(null, true)");
    assert_eq!(SessionFilter::Regular.to_string(), "SessionFilter(REGULAR, null)");
}
