//! Usage: `cargo run --example IpfLiveSample -- [<ipf-url>]`

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use dxfeed_graal::TimeFormat;
use dxfeed_graal::ipf::InstrumentProfile;
use dxfeed_graal::ipf::live::{InstrumentProfileCollector, InstrumentProfileConnection};

const DXFEED_IPF_URL: &str = "https://demo:demo@tools.dxfeed.com/ipf";

fn print_usage() {
    let usage_string = format!(
        r#"
Usage:
IpfLiveSample [<ipf-url>]

Where:
    <ipf-url> - URL for the instruments profiles, default: {DXFEED_IPF_URL}"#
    );

    println!("{usage_string}");
}

fn main() -> dxfeed_graal::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() > 1 && (args[1] == "-h" || args[1] == "--help") {
        print_usage();

        return Ok(());
    }

    let url = if args.len() > 1 {
        args[1].as_str()
    } else {
        DXFEED_IPF_URL
    };
    let collector = InstrumentProfileCollector::new()?;
    let connection = InstrumentProfileConnection::create_connection(url, &collector)?;

    // Update period can be used to re-read IPF files, not needed for services supporting IPF "live-update"
    connection.set_update_period(Duration::from_secs(60))?;

    connection.add_state_change_listener(|_old_state, new_state| {
        println!("Connection state: {new_state}");
    })?;

    connection.start()?;

    // We can wait until we get the first full snapshot of instrument profiles
    connection.wait_until_completed(Duration::from_secs(10))?;

    // Data model to keep all instrument profiles mapped by their ticker symbol
    let mut profiles: HashMap<String, InstrumentProfile> = HashMap::new();

    // It is possible to add listener after connection is started - updates will not be missed in this case
    // The listener holds a clone of the collector (there is no weak reference to it), so it is removed below
    let self_collector = collector.clone();
    let listener_id = collector.add_update_listener(move |instruments| {
        println!("\nInstrument Profiles:");
        // We can observe REMOVED elements - need to add necessary filtering
        // See documentation for `InstrumentProfileCollector` for more details

        // (1) We can either process instrument profile updates manually
        for profile in instruments {
            if profile.instrument_type == InstrumentProfile::REMOVED {
                // Profile was removed - remove it from our data model
                profiles.remove(&profile.symbol);
            } else {
                // Profile was updated - collector only notifies us if the profile was changed
                profiles.insert(profile.symbol.clone(), profile.clone());
            }
        }

        println!("Total number of profiles (1): {}", profiles.len());

        // (2) or access the concurrent view of instrument profiles
        let mut symbols: HashSet<String> = HashSet::new();

        for ip in self_collector.view().unwrap() {
            if ip.instrument_type != InstrumentProfile::REMOVED {
                symbols.insert(ip.symbol);
            }
        }

        println!("Total number of profiles (2): {}", symbols.len());

        println!(
            "Last modified: {}",
            TimeFormat::DEFAULT
                .with_millis()
                .with_time_zone()
                .format(self_collector.get_last_update_time().unwrap())
                .unwrap()
        );
    })?;

    std::thread::sleep(Duration::from_secs(365 * 24 * 60 * 60));

    collector.remove_update_listener(listener_id)?;
    connection.close()?;

    Ok(())
}
