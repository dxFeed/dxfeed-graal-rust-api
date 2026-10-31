//! Shows the trading schedule of an instrument at a time: the next holidays, the current session, the next and the
//! nearest trading sessions, like `ScheduleSample` of the other dxFeed APIs.
//!
//! Usage: `cargo run --example ScheduleSample -- <DEFAULTS> <PROFILES> <SYMBOL> [TIME]`; for example, in the
//! directory of the sample: `schedule.properties sample.ipf.zip AAPL 2026-10-28-14:15:00`.

use std::collections::HashMap;
use std::process::ExitCode;

use dxfeed_graal::TimeFormat;
use dxfeed_graal::ipf::{InstrumentProfile, InstrumentProfileReader};
use dxfeed_graal::schedule::{DayFilter, Schedule, SessionFilter};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (defaults, profiles, symbol, time) = match args.as_slice() {
        [defaults, profiles, symbol] => (defaults, profiles, symbol, None),
        [defaults, profiles, symbol, time] => (defaults, profiles, symbol, Some(time.as_str())),
        _ => {
            eprintln!("Usage: ScheduleSample <defaults> <profiles> <symbol> [<time>]");
            eprintln!("  defaults - the path of the Schedule API defaults file");
            eprintln!("  profiles - the file or the URL of the instrument profiles");
            eprintln!("  symbol   - the symbol of the instrument");
            eprintln!("  time     - the time in the format yyyy-MM-dd-HH:mm:ss; now by default");
            eprintln!("For example: ScheduleSample schedule.properties sample.ipf.zip AAPL 2026-10-28-14:15:00");
            return ExitCode::from(2);
        }
    };
    match run(defaults, profiles, symbol, time) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(defaults: &str, profiles: &str, symbol: &str, time: Option<&str>) -> dxfeed_graal::Result<()> {
    update_schedule_defaults(defaults)?;
    let profiles = load_instrument_profiles(profiles)?;
    check_all_schedules(&profiles);

    let Some(profile) = profiles.get(symbol) else {
        println!("Could not find profile for {symbol}");
        return Ok(());
    };
    println!("Found profile for {symbol}: {}", profile.description);

    let time = match time {
        Some(time) => TimeFormat::DEFAULT.parse(time)?,
        None => now(),
    };
    println!(
        "Using timestamp {}",
        TimeFormat::DEFAULT.with_millis().with_time_zone().format(time)?
    );

    print_next_5_holidays(profile, time)?;
    print_current_session(profile, time)?;
    print_next_trading_session(profile, time)?;
    print_nearest_trading_session(profile, time)
}

fn update_schedule_defaults(path: &str) -> dxfeed_graal::Result<()> {
    let data = std::fs::read(path)
        .map_err(|error| dxfeed_graal::Error::InvalidArgument(format!("cannot read {path}: {error}")))?;
    Schedule::set_defaults(&data)
}

fn load_instrument_profiles(url: &str) -> dxfeed_graal::Result<HashMap<String, InstrumentProfile>> {
    let profiles: HashMap<String, InstrumentProfile> = InstrumentProfileReader::new()?
        .read_from_file(url)?
        .into_iter()
        .map(|profile| (profile.symbol.clone(), profile))
        .collect();
    println!("Loaded {} instrument profiles", profiles.len());
    Ok(profiles)
}

fn check_all_schedules(profiles: &HashMap<String, InstrumentProfile>) {
    let check = |profile: &InstrumentProfile| -> dxfeed_graal::Result<()> {
        Schedule::get_instance(profile)?;
        for venue in Schedule::get_trading_venues(profile)? {
            Schedule::get_instance_with_venue(profile, &venue)?;
        }
        Ok(())
    };
    let mut successes = 0;
    for profile in profiles.values() {
        match check(profile) {
            Ok(()) => successes += 1,
            Err(error) => {
                println!(
                    "Error getting schedule for {} ({}): {error}",
                    profile.symbol, profile.trading_hours
                )
            }
        }
    }
    println!(
        "Checked {} instrument profiles: {successes} successes, {} failures",
        profiles.len(),
        profiles.len() - successes
    );
}

fn print_next_5_holidays(profile: &InstrumentProfile, time: i64) -> dxfeed_graal::Result<()> {
    let mut day = Schedule::get_instance(profile)?.get_day_by_time(time)?;
    let mut output = format!("5 next holidays for {}:", profile.symbol);
    for _ in 0..5 {
        match day.find_next_day(DayFilter::Holiday)? {
            Some(next) => day = next,
            None => break,
        }
        output += &format!(" {}", day.year_month_day()?);
    }
    println!("{output}");
    Ok(())
}

fn print_current_session(profile: &InstrumentProfile, time: i64) -> dxfeed_graal::Result<()> {
    let session = Schedule::get_instance(profile)?.get_session_by_time(time)?;
    println!(
        "Current session for {}: {session} in {}",
        profile.symbol,
        session.get_day()?
    );
    Ok(())
}

fn print_next_trading_session(profile: &InstrumentProfile, time: i64) -> dxfeed_graal::Result<()> {
    let mut session = Schedule::get_instance(profile)?.get_session_by_time(time)?;
    if !session.is_trading()? {
        session = session.get_next_session(SessionFilter::Trading)?;
    }
    println!(
        "Next trading session for {}: {session} in {}",
        profile.symbol,
        session.get_day()?
    );
    Ok(())
}

fn print_nearest_trading_session(profile: &InstrumentProfile, time: i64) -> dxfeed_graal::Result<()> {
    let session = Schedule::get_instance(profile)?.get_nearest_session_by_time(time, SessionFilter::Trading)?;
    println!(
        "Nearest trading session for {}: {session} in {}",
        profile.symbol,
        session.get_day()?
    );
    Ok(())
}

/// The current time in milliseconds since the Unix epoch.
fn now() -> i64 {
    let since_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    i64::try_from(since_epoch.as_millis()).unwrap_or(i64::MAX)
}
