//! Reads the instrument profiles of a file, selects the stocks of computer and office equipment traded at NYSE,
//! subscribes to the events of the given types for them and prints the events until Enter is pressed, like
//! `IpfSample` of the other dxFeed APIs. It uses the default feed, whose address is the `dxfeed.address` system
//! property.
//!
//! Usage: `cargo run --example ipf -- <TYPES> <IPF_FILE> [ADDRESS]`, for example
//! `Quote https://dxfeed.s3.amazonaws.com/masterdata/ipf/demo/mux-demo.ipf.zip`. Without ADDRESS the
//! address is `dxfeed.address` of the `dxfeed.system.properties` file in the current directory or of the
//! `DXFEED_dxfeed.address` environment variable, else `demo.dxfeed.com:7300`.

use std::process::ExitCode;

use dxfeed_graal::ipf::{InstrumentProfile, InstrumentProfileReader};
use dxfeed_graal::{DXEndpoint, DXFeed, EventType, system};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (types, file, address) = match args.as_slice() {
        [types, file] => (types, file, None),
        [types, file, address] => (types, file, Some(address.clone())),
        _ => {
            eprintln!("Usage: ipf <types> <ipf-file> [<address>]");
            eprintln!("  types    - comma-separated event types, for example Quote,Trade");
            eprintln!("  ipf-file - the file or the URL of the instrument profiles, for example");
            eprintln!("             https://dxfeed.s3.amazonaws.com/masterdata/ipf/demo/mux-demo.ipf.zip");
            eprintln!("  address  - the address to connect to; dxfeed.address of dxfeed.system.properties by default");
            return ExitCode::from(2);
        }
    };
    match run(types, file, address) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(types: &str, file: &str, address: Option<String>) -> dxfeed_graal::Result<()> {
    let types = parse_types(types)?;
    let symbols = symbols(file)?;

    // The default feed connects to `dxfeed.address` when it is first used.
    system::set_property("dxfeed.address", &address_or_default(address)?)?;
    let subscription = DXFeed::get_instance()?.create_subscription_with_types(&types)?;
    subscription.add_event_listener(|events| {
        for event in events {
            println!("{event}");
        }
    })?;
    subscription.add_symbols(&symbols)?;
    eprintln!("Press Enter to stop.");

    // Enter or the end of input stops the program.
    let _ = std::io::stdin().read_line(&mut String::new());
    DXEndpoint::get_instance()?.close_and_await_termination()
}

/// The symbols of the selected instruments of a file.
fn symbols(file: &str) -> dxfeed_graal::Result<Vec<String>> {
    println!("Reading instruments from {file}...");
    let profiles = InstrumentProfileReader::new()?.read_from_file(file)?;
    // This is just a sample, any arbitrary filtering may go here.
    let selected = |profile: &InstrumentProfile| {
        profile.instrument_type == "STOCK" // stocks
            && profile.sic / 10 == 357 // Computer And Office Equipment
            && profile.exchanges.contains("XNYS") // traded at NYSE
    };
    println!("Selected symbols are:");
    let mut symbols = Vec::new();
    for profile in profiles.into_iter().filter(selected) {
        println!("{} ({})", profile.symbol, profile.description);
        symbols.push(profile.symbol);
    }
    Ok(symbols)
}

/// The event types of comma-separated names in any case.
fn parse_types(types: &str) -> dxfeed_graal::Result<Vec<EventType>> {
    types
        .split(',')
        .map(|name| {
            EventType::ALL
                .iter()
                .copied()
                .find(|t| t.name().eq_ignore_ascii_case(name.trim()))
                .ok_or_else(|| dxfeed_graal::Error::InvalidArgument(format!("unknown event type {name:?}")))
        })
        .collect()
}

/// The ADDRESS argument; without it, `dxfeed.address` of the `dxfeed.system.properties` file or of the
/// `DXFEED_dxfeed.address` environment variable, and the demo feed as the last resort.
fn address_or_default(address: Option<String>) -> dxfeed_graal::Result<String> {
    match address {
        Some(address) => Ok(address),
        None => Ok(system::get_property("dxfeed.address")?.unwrap_or_else(|| "demo.dxfeed.com:7300".into())),
    }
}
