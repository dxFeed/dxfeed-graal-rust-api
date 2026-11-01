//! This sample demonstrates how to subscribe to available symbols using IPF.
//! Use the default DXFeed instance for that data feed address is defined by "dxfeed.system.properties" file.
//! The properties file is read from the current directory, so run the sample from its directory:
//! `cargo run --example IpfSample -- <type> <ipf-file>`.

use std::time::Duration;

use dxfeed_graal::ipf::{InstrumentProfile, InstrumentProfileReader};
use dxfeed_graal::{DXFeed, EventType};

fn print_usage() {
    let usage_string = format!(
        r#"
Usage:
IpfSample <type> <ipf-file>

Where:
    type     - Is dxfeed event type ({names} or {class_names}).
    ipf-file - Is name of instrument profiles file.

Examples:
    IpfSample Quote https://dxfeed.s3.amazonaws.com/masterdata/ipf/demo/mux-demo.ipf.zip
"#,
        // Rust has no market flag, so all event types are listed.
        names = get_event_type_enum_names_list(),
        class_names = get_event_type_enum_class_names_list(),
    );

    println!("{usage_string}");
}

fn get_symbols(filename: &str) -> dxfeed_graal::Result<Vec<String>> {
    println!("Reading instruments from {filename}...");

    let profiles = InstrumentProfileReader::new()?.read_from_file(filename)?;
    let profile_filter = |profile: &InstrumentProfile| -> bool {
        // This is just a sample, any arbitrary filtering may go here.
        profile.instrument_type == "STOCK" // stocks
            && profile.sic / 10 == 357 // Computer And Office Equipment
            && profile.exchanges.contains("XNYS") // traded at NYSE
    };

    let mut result = Vec::new();

    println!("Selected symbols are:");

    for profile in profiles {
        if profile_filter(&profile) {
            println!("{} ({})", profile.symbol, profile.description);
            result.push(profile.symbol);
        }
    }

    Ok(result)
}

fn main() -> dxfeed_graal::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 3 {
        print_usage();

        return Ok(());
    }

    let (parsed_types, _unknown_types) = parse_types(&args[1]);
    let ipf_file = &args[2];
    let sub = DXFeed::get_instance()?.create_subscription_with_types(&parsed_types)?;

    sub.add_event_listener(|events| {
        for event in events {
            // Java's toString() of the event already contains the symbol.
            println!("{event}");
        }
    })?;

    sub.add_symbols(get_symbols(ipf_file)?)?;

    std::thread::sleep(Duration::from_secs(365 * 24 * 60 * 60));

    Ok(())
}

/// The names of all event types (QUOTE, TIME_AND_SALE etc.) in alphabetical order, comma-separated.
fn get_event_type_enum_names_list() -> String {
    let mut names: Vec<String> = EventType::ALL
        .iter()
        .map(|t| to_upper_snake_case(t.get_name()))
        .collect();
    names.sort();

    names.join(",")
}

/// The class names of all event types (Candle, Quote, TimeAndSale etc.) in alphabetical order, comma-separated.
fn get_event_type_enum_class_names_list() -> String {
    let mut names: Vec<&str> = EventType::ALL.iter().map(|t| t.get_name()).collect();
    names.sort();

    names.join(",")
}

/// `TimeAndSale` -> `TIME_AND_SALE`.
fn to_upper_snake_case(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut result = String::new();

    for (i, &c) in chars.iter().enumerate() {
        // A word starts at an uppercase letter that follows a lowercase one, or that ends a run of uppercase letters.
        if i > 0
            && c.is_ascii_uppercase()
            && (chars[i - 1].is_ascii_lowercase() || chars.get(i + 1).is_some_and(|n| n.is_ascii_lowercase()))
        {
            result.push('_');
        }

        result.push(c.to_ascii_uppercase());
    }

    result
}

/// The event types of a comma-separated list of names (QUOTE) or class names (Quote), without repeats, and the unknown
/// names; `all`, `All`, `ALL` or `*` for all the types.
fn parse_types(types: &str) -> (Vec<EventType>, Vec<String>) {
    let types = types.trim();

    if types.is_empty() {
        return Default::default();
    }

    if matches!(types, "*" | "all" | "All" | "ALL") {
        return (EventType::ALL.to_vec(), Vec::new());
    }

    let (mut result, mut unknown) = (Vec::new(), Vec::new());

    for t in types.split(',').map(str::trim).filter(|t| !t.is_empty()) {
        let found = EventType::ALL
            .iter()
            .copied()
            .find(|e| to_upper_snake_case(e.get_name()) == t.to_uppercase() || e.get_name() == t);

        match found {
            Some(event_type) if !result.contains(&event_type) => result.push(event_type),
            Some(_) => {}
            None => unknown.push(t.to_owned()),
        }
    }

    (result, unknown)
}
