//! Follows the instrument profiles of a service with live updates, or of a file read again when it changes, and
//! prints the number of profiles after every update until Enter is pressed, like `IpfLiveSample` of the other dxFeed
//! APIs.
//!
//! Usage: `cargo run --example ipf_live -- [IPF_URL]`; the default is `https://demo:demo@tools.dxfeed.com/ipf`.

use std::collections::HashMap;
use std::process::ExitCode;
use std::time::Duration;

use dxfeed_graal::TimeFormat;
use dxfeed_graal::ipf::InstrumentProfile;
use dxfeed_graal::ipf::live::{InstrumentProfileCollector, InstrumentProfileConnection};

const DXFEED_IPF_URL: &str = "https://demo:demo@tools.dxfeed.com/ipf";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let url = match args.as_slice() {
        [] => DXFEED_IPF_URL,
        [url] if url != "-h" && url != "--help" => url.as_str(),
        _ => {
            eprintln!("Usage: ipf_live [<ipf-url>]");
            eprintln!("  ipf-url - the URL or the file of the instrument profiles, {DXFEED_IPF_URL} by default");
            return ExitCode::from(2);
        }
    };
    match run(url) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(url: &str) -> dxfeed_graal::Result<()> {
    let collector = InstrumentProfileCollector::new()?;
    let connection = InstrumentProfileConnection::create_connection(url, &collector)?;
    // The update period makes the connection read a file again; services with live updates do not need it.
    connection.set_update_period(Duration::from_secs(60))?;
    connection.add_state_change_listener(|_, state| println!("Connection state: {state}"))?;
    connection.start()?;
    // The first full snapshot of the profiles can be waited for.
    connection.wait_until_completed(Duration::from_secs(10))?;

    // A listener added after the start misses no updates: it gets every profile first.
    let mut profiles: HashMap<String, InstrumentProfile> = HashMap::new();
    let view = collector.clone();
    let id = collector.add_update_listener(move |updated| {
        println!();
        println!("Instrument Profiles:");
        // Either follow the updates, removed profiles included...
        for profile in updated {
            if profile.is_removed() {
                profiles.remove(&profile.symbol);
            } else {
                profiles.insert(profile.symbol.clone(), profile.clone());
            }
        }
        println!("Total number of profiles (1): {}", profiles.len());
        // ...or look at the profiles in the collector.
        match view.view() {
            Ok(all) => println!(
                "Total number of profiles (2): {}",
                all.iter().filter(|profile| !profile.is_removed()).count()
            ),
            Err(error) => eprintln!("error: {error}"),
        }
        match view
            .get_last_update_time()
            .and_then(|time| TimeFormat::DEFAULT.with_millis().with_time_zone().format(time))
        {
            Ok(time) => println!("Last modified: {time}"),
            Err(error) => eprintln!("error: {error}"),
        }
    })?;
    eprintln!("Press Enter to stop.");

    // Enter or the end of input stops the program.
    let _ = std::io::stdin().read_line(&mut String::new());
    // The listener holds a clone of the collector: removing it lets the collector go.
    collector.remove_update_listener(id)?;
    connection.close()
}
