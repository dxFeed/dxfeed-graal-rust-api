//! Usage: `cargo run --example PriceLevelServiceClientSample -- <address> <symbol> <source> <from> <to> [--validate]
//! [--verbose]`

use std::time::{Duration, Instant};

use dxfeed_graal::event::{CandleSymbol, OrderSource};
use dxfeed_graal::orcs::{PriceLevelChecker, PriceLevelService};
use dxfeed_graal::{TimeFormat, system};

fn print_usage() {
    let usage_string = r#"
Usage:
PriceLevelServiceClientSample <address> <symbol> <source> <from> <to> [--validate] [--verbose]

Where:
    address    - The RMI address of the ORCS server. To pass an authorization token, add "[login=entitle:<token>]" to
                 the address: "<address>[login=entitle:<token>]".
    symbol     - The candle symbol to retrieve, for example "AAPL{=d,gr=s}" (the "gr" attribute is the granularity of
                 the price levels in seconds).
    source     - The order source to retrieve, for example "NTV".
    from       - Is from-time for the request in standard formats.
                 Some examples of valid from-time:
                     20070101-123456
                     20070101-123456.123
                     2005-12-31 21:00:00
                     2005-12-31 21:00:00.123+03:00
                     2005-12-31 21:00:00.123+0400
                     2007-11-02Z
                     123456789 - value-in-milliseconds
    to         - Is to-time for the request in standard formats.
    --validate - Validate the received price levels with PriceLevelChecker (the details go to the QD log).
    --verbose  - Print the available symbols of each authorized order source.

Examples:
    PriceLevelServiceClientSample "orcs.example.com:7777[login=entitle:<token>]" "AAPL{=d,gr=s}" NTV 20260110-100000 20260110-230000 --validate"#;

    println!("{usage_string}");
}

fn main() -> dxfeed_graal::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 6 {
        print_usage();

        return Ok(());
    }

    // Parse args.
    let address = &args[1];
    let symbol = CandleSymbol::value_of(&args[2])?;
    let source = OrderSource::value_of(&args[3])?;
    let from_time = TimeFormat::DEFAULT.parse(&args[4])?;
    let to_time = TimeFormat::DEFAULT.parse(&args[5])?;
    let mut validate = false;
    let mut verbose = false;

    for arg in &args[6..] {
        if arg == "--validate" {
            validate = true;
        } else if arg == "--verbose" {
            verbose = true;
        }
    }

    // Disable QD logging, unless PriceLevelChecker logs the validation to it.
    if !validate {
        system::set_property("log.className", "com.devexperts.logging.InterceptableLogging")?;
        system::set_property("log.level", "OFF")?;
        system::set_property("err.level", "OFF")?;
    }

    let service = PriceLevelService::new(address)?;
    let mut symbols_by_source: Vec<_> = service
        .get_auth_order_source()?
        .get_by_order_sources()?
        .into_iter()
        .collect();
    // The order of the sources is the order of their names.
    symbols_by_source.sort_by_key(|(order_source, _)| order_source.to_string());

    print!("Authorized sources: [");

    for (i, (order_source, _symbols)) in symbols_by_source.iter().enumerate() {
        print!("{}{order_source}", if i == 0 { "" } else { ", " });
    }

    println!("]");

    if verbose {
        for (order_source, symbols) in &symbols_by_source {
            print!("{order_source}: [");

            for (i, s) in symbols.iter().enumerate() {
                print!("{}{s}", if i == 0 { "" } else { ", " });
            }

            println!("]");
        }
    }

    let mut start = Instant::now();
    let orders = service.get_orders(&symbol, source, from_time, to_time, Some("sample"))?;

    println!("Received {} orders in {} ms", orders.len(), start.elapsed().as_millis());

    if validate {
        let valid = PriceLevelChecker::validate(&orders, Duration::from_secs(60), true)?;

        println!("The orders are {}", if valid { "valid" } else { "not valid" });
    }

    start = Instant::now();
    let quotes = service.get_quotes(
        &CandleSymbol::value_of(&format!(
            "{}{{{}=s}}",
            base_symbol(symbol.as_str()),
            PriceLevelService::GRANULARITY_ATTRIBUTE_KEY
        ))?,
        from_time,
        to_time,
        Some("sample"),
    )?;

    println!("Received {} quotes in {} ms", quotes.len(), start.elapsed().as_millis());

    service.close()?;

    Ok(())
}

/// The symbol without its attributes and its exchange code, such as `AAPL` for `AAPL&Q{=d,gr=s}`, like
/// `MarketEventSymbols.getBaseSymbol`.
fn base_symbol(symbol: &str) -> &str {
    let symbol = match symbol.rfind('{') {
        Some(i) if symbol.ends_with('}') => &symbol[..i],
        _ => symbol,
    };
    match symbol.char_indices().rev().nth(1) {
        Some((i, '&')) => &symbol[..i],
        _ => symbol,
    }
}
