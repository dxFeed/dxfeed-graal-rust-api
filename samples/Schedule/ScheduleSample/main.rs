//! A sample program that demonstrates different use cases of Schedule API.
//!
//! Usage: `cargo run --bin ScheduleSample -- <defaults> <profiles> <symbol> [time]`, run from the sample
//! directory when the sample files `schedule.properties` and `sample.ipf.zip` are used.

use std::collections::HashMap;

use dxfeed_graal::TimeFormat;
use dxfeed_graal::ipf::{InstrumentProfile, InstrumentProfileReader};
use dxfeed_graal::schedule::{DayFilter, Schedule, SessionFilter};

fn print_usage() {
    println!(
        r#"usage:  ScheduleSample  <defaults>  <profiles>  <symbol>  [time]
where:  <defaults>  is a path to Schedule API defaults file
        <profiles>  is a URL to IPF file
        <symbol>    is a ticker symbol used for sample
        [time]      is a time used for sample in a format yyyy-MM-dd-HH:mm:ss
sample: ScheduleSample schedule.properties sample.ipf.zip IBM 2011-05-26-14:15:00"#
    );
}

fn update_schedule_defaults(url: &str) -> dxfeed_graal::Result<()> {
    let Ok(buffer) = std::fs::read(url) else {
        eprintln!("Failed to open {url}");

        return Ok(());
    };

    Schedule::set_defaults(&buffer)
}

fn load_instrument_profiles(url: &str) -> dxfeed_graal::Result<HashMap<String, InstrumentProfile>> {
    let mut profiles = HashMap::new();

    for profile in InstrumentProfileReader::new()?.read_from_file(url)? {
        profiles.insert(profile.symbol.clone(), profile);
    }

    println!("Loaded {} instrument profiles", profiles.len());

    Ok(profiles)
}

fn check_all_schedules(profiles: &HashMap<String, InstrumentProfile>) -> dxfeed_graal::Result<()> {
    let mut successes = 0;

    for profile in profiles.values() {
        let mut error = false;

        if Schedule::get_instance(profile).is_err() {
            error = true;
        } else {
            for venue in Schedule::get_trading_venues(profile)? {
                if Schedule::get_instance_with_venue(profile, &venue).is_err() {
                    error = true;
                    break;
                }
            }
        }

        if error {
            eprintln!(
                "Error getting schedule for {} ({}): ",
                profile.symbol, profile.trading_hours
            );
        } else {
            successes += 1;
        }
    }

    println!(
        "Checked {} instrument profiles: {successes} successes, {} failures",
        profiles.len(),
        profiles.len() - successes
    );

    Ok(())
}

fn print_next_5_holidays(profile: &InstrumentProfile, time: i64) -> dxfeed_graal::Result<()> {
    let schedule = Schedule::get_instance(profile)?;
    let day = schedule.get_day_by_time(time);

    let mut output = format!("5 next holidays for {}:", profile.symbol);

    match day {
        Err(_) => output += "Could not get day by time",
        Ok(mut day) => {
            for _ in 0..5 {
                match day.find_next_day(DayFilter::Holiday)? {
                    Some(next_day) => day = next_day,
                    None => break,
                }

                output += &format!(" {}", day.get_year_month_day()?);
            }
        }
    }

    println!("{output}");

    Ok(())
}

fn print_current_session(profile: &InstrumentProfile, time: i64) -> dxfeed_graal::Result<()> {
    let Ok(schedule) = Schedule::get_instance(profile) else {
        eprintln!(
            "Error getting schedule for {} ({}): ",
            profile.symbol, profile.trading_hours
        );

        return Ok(());
    };

    let Ok(session) = schedule.get_session_by_time(time) else {
        eprintln!(
            "Error getting session for time: {}",
            TimeFormat::DEFAULT.with_millis().with_time_zone().format(time)?
        );

        return Ok(());
    };

    println!(
        "Current session for {}: {session} in {}",
        profile.symbol,
        session.get_day()?
    );

    Ok(())
}

fn print_next_trading_session(profile: &InstrumentProfile, time: i64) -> dxfeed_graal::Result<()> {
    let Ok(schedule) = Schedule::get_instance(profile) else {
        eprintln!(
            "Error getting schedule for {} ({}): ",
            profile.symbol, profile.trading_hours
        );

        return Ok(());
    };

    let Ok(session) = schedule.get_session_by_time(time) else {
        eprintln!(
            "Error getting session for time: {}",
            TimeFormat::DEFAULT.with_millis().with_time_zone().format(time)?
        );

        return Ok(());
    };

    // The next trading session may not exist.
    let session = if session.is_trading()? {
        Some(session)
    } else {
        session.find_next_session(SessionFilter::Trading)?
    };

    match session {
        None => eprintln!("There is no next trading session for {}", profile.symbol),
        Some(session) => println!(
            "Next trading session for {}: {session} in {}",
            profile.symbol,
            session.get_day()?
        ),
    }

    Ok(())
}

fn print_nearest_trading_session(profile: &InstrumentProfile, time: i64) -> dxfeed_graal::Result<()> {
    let Ok(schedule) = Schedule::get_instance(profile) else {
        eprintln!(
            "Error getting schedule for {} ({}): ",
            profile.symbol, profile.trading_hours
        );

        return Ok(());
    };

    match schedule.find_nearest_session_by_time(time, SessionFilter::Trading)? {
        None => eprintln!("There is no nearest trading session for {}", profile.symbol),
        Some(session) => println!(
            "Nearest trading session for {}: {session} in {}",
            profile.symbol,
            session.get_day()?
        ),
    }

    Ok(())
}

fn main() -> dxfeed_graal::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() > 1 && (args[1] == "-h" || args[1] == "--help") {
        print_usage();

        return Ok(());
    }

    if args.len() < 4 || args.len() > 5 {
        eprintln!("Wrong number of arguments");
        print_usage();

        std::process::exit(1);
    }

    update_schedule_defaults(&args[1])?;

    let profiles = load_instrument_profiles(&args[2])?;

    check_all_schedules(&profiles)?;

    let symbol = &args[3];

    let Some(profile) = profiles.get(symbol) else {
        eprintln!("Could not find profile for {symbol}");

        std::process::exit(2);
    };

    println!("Found profile for {symbol}: {}", profile.description);

    let time = if args.len() < 5 {
        now()
    } else {
        TimeFormat::DEFAULT.with_millis().with_time_zone().parse(&args[4])?
    };

    println!(
        "Using timestamp {}",
        TimeFormat::DEFAULT.with_millis().with_time_zone().format(time)?
    );

    print_next_5_holidays(profile, time)?;
    print_current_session(profile, time)?;
    print_next_trading_session(profile, time)?;
    print_nearest_trading_session(profile, time)?;

    Ok(())
}

/// The current time in milliseconds since the Unix epoch.
fn now() -> i64 {
    let since_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();

    i64::try_from(since_epoch.as_millis()).unwrap_or(i64::MAX)
}
