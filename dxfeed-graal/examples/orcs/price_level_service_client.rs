//! Gets the price levels (orders) and the quotes of a candle symbol for a period from the Order Reconstruction Service
//! (ORCS), like `PriceLevelServiceClientSample` of the other dxFeed APIs, and optionally validates the price levels.
//!
//! Usage: `cargo run --example price_level_service_client -- <ADDRESS> <SYMBOL> <SOURCE> <FROM> <TO> [--validate]
//! [--verbose]`; run it without arguments for the details.

use std::process::ExitCode;
use std::time::{Duration, Instant};

use dxfeed_graal::TimeFormat;
use dxfeed_graal::event::{CandleSymbol, OrderSource};
use dxfeed_graal::orcs::{PriceLevelChecker, PriceLevelService};

const USAGE: &str = r#"
Usage:
price_level_service_client <address> <symbol> <source> <from> <to> [--validate] [--verbose]

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
    price_level_service_client "orcs.example.com:7777[login=entitle:<token>]" "AAPL{=d,gr=s}" NTV 20260110-100000 20260110-230000 --validate"#;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 5 {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> dxfeed_graal::Result<()> {
    let address = &args[0];
    let symbol = CandleSymbol::value_of(&args[1])?;
    let source = OrderSource::value_of(&args[2])?;
    let from_time = TimeFormat::DEFAULT.parse(&args[3])?;
    let to_time = TimeFormat::DEFAULT.parse(&args[4])?;
    let validate = args[5..].iter().any(|arg| arg == "--validate");
    let verbose = args[5..].iter().any(|arg| arg == "--verbose");

    let service = PriceLevelService::new(address)?;
    let mut symbols_by_source: Vec<_> = service
        .get_auth_order_source()?
        .get_by_order_sources()?
        .into_iter()
        .collect();
    symbols_by_source.sort_by_key(|(source, _)| source.to_string());
    let sources: Vec<_> = symbols_by_source.iter().map(|(source, _)| source.to_string()).collect();
    println!("Authorized sources: [{}]", sources.join(", "));
    if verbose {
        for (source, symbols) in &symbols_by_source {
            println!("{source}: [{}]", symbols.join(", "));
        }
    }

    let start = Instant::now();
    let orders = service.get_orders(&symbol, source, from_time, to_time, "sample")?;
    println!("Received {} orders in {} ms", orders.len(), start.elapsed().as_millis());
    if validate {
        let valid = PriceLevelChecker::validate(&orders, Duration::from_secs(60), true)?;
        println!("The orders are {}", if valid { "valid" } else { "not valid" });
    }

    // The quotes of the base symbol with the granularity of a second.
    let base_symbol = base_symbol(symbol.as_str());
    let start = Instant::now();
    let quotes = service.get_quotes(
        &CandleSymbol::value_of(&format!("{base_symbol}{{gr=s}}"))?,
        from_time,
        to_time,
        "sample",
    )?;
    println!("Received {} quotes in {} ms", quotes.len(), start.elapsed().as_millis());

    service.close()
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
