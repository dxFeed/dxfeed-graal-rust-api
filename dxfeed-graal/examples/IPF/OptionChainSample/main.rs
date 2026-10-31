//! Builds the option chains of a file of instrument profiles and prints the quotes of the options with strikes
//! around the last price of a symbol, like `OptionChainSample` of the other dxFeed APIs. It uses the default feed,
//! whose address is the `dxfeed.address` system property.
//!
//! Usage: `cargo run --example OptionChainSample -- <IPF_FILE> <SYMBOL> <N_STRIKES> <N_MONTHS> [ADDRESS]`, for example
//! `https://demo:demo@tools.dxfeed.com/ipf AAPL 5 5`. Without ADDRESS the address is `dxfeed.address` of the
//! `dxfeed.system.properties` file in the current directory or of the `DXFEED_dxfeed.address` environment variable,
//! else `demo.dxfeed.com:7300`.

use std::collections::HashMap;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use dxfeed_graal::event::{Quote, Trade};
use dxfeed_graal::ipf::option::{OptionChainsBuilder, OptionSeries};
use dxfeed_graal::ipf::{InstrumentProfile, InstrumentProfileReader};
use dxfeed_graal::{DXEndpoint, DXFeed, Error, Promise, system};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let parsed = match args.as_slice() {
        [file, symbol, strikes, months] => parse(file, symbol, strikes, months, None),
        [file, symbol, strikes, months, address] => parse(file, symbol, strikes, months, Some(address.clone())),
        _ => None,
    };
    let Some(args) = parsed else {
        eprintln!("Usage: option_chain <ipf-file> <symbol> <n-strikes> <n-months> [<address>]");
        eprintln!("  ipf-file  - the file or the URL of the instrument profiles, for example");
        eprintln!("              https://demo:demo@tools.dxfeed.com/ipf");
        eprintln!("  symbol    - the product or underlying symbol, for example AAPL");
        eprintln!("  n-strikes - the number of strikes to print for each series");
        eprintln!("  n-months  - the number of months to print");
        eprintln!("  address   - the address to connect to; dxfeed.address of dxfeed.system.properties by default");
        return ExitCode::from(2);
    };
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

struct Args {
    file: String,
    symbol: String,
    n_strikes: usize,
    n_months: usize,
    address: Option<String>,
}

fn parse(file: &str, symbol: &str, strikes: &str, months: &str, address: Option<String>) -> Option<Args> {
    Some(Args {
        file: file.to_owned(),
        symbol: symbol.to_owned(),
        n_strikes: strikes.parse().ok()?,
        n_months: months.parse().ok()?,
        address,
    })
}

fn run(args: Args) -> dxfeed_graal::Result<()> {
    // The default feed connects to `dxfeed.address` when it is first used.
    system::set_property("dxfeed.address", &address_or_default(args.address)?)?;
    let feed = DXFeed::get_instance()?;

    // Subscribe to Trade to learn the last price of the instrument.
    println!("Waiting for price of {} ...", args.symbol);
    let trade = feed
        .get_last_event_promise::<Trade>(&args.symbol)?
        .wait(Duration::from_secs(1))?;
    let price = trade.price;
    println!("Price of {} is {price:?}", args.symbol);

    println!("Reading instruments from {} ...", args.file);
    let instruments = InstrumentProfileReader::new()?.read_from_file(&args.file)?;

    println!("Building option chains ...");
    let chains = OptionChainsBuilder::build(&instruments).into_chains();
    let chain = chains
        .get(&args.symbol)
        .ok_or_else(|| Error::InvalidArgument(format!("no option chain of {}", args.symbol)))?;
    let series = &chain.get_series()[..args.n_months.min(chain.get_series().len())];

    println!("Requesting option quotes ...");
    let mut quotes: HashMap<String, Promise<Quote>> = HashMap::new();
    for s in series {
        for option in options_around(s, args.n_strikes, price) {
            quotes.insert(
                option.symbol.clone(),
                feed.get_last_event_promise::<Quote>(&option.symbol)?,
            );
        }
    }
    // Ignore the timeout and print the quotes that came.
    let deadline = Instant::now() + Duration::from_secs(10);
    for promise in quotes.values() {
        let _ = promise.wait(deadline.saturating_duration_since(Instant::now()));
    }

    println!("Printing option series ...");
    let quote = |option: Option<&&InstrumentProfile>| {
        option
            .and_then(|option| quotes.get(&option.symbol))
            .and_then(|promise| promise.get_result().ok().flatten())
            .unwrap_or_else(|| Quote::new(""))
    };
    for s in series {
        println!("Option series {s}");
        println!(
            "    {:>10} {:>10} {:>10} {:>10} {:>10}",
            "C.BID", "C.ASK", "STRIKE", "P.BID", "P.ASK"
        );
        for &strike in s.get_n_strikes_around(args.n_strikes, price) {
            let call = quote(s.call(strike));
            let put = quote(s.put(strike));
            println!(
                "    {:10.3} {:10.3} {:10.3} {:10.3} {:10.3}",
                call.bid_price, call.ask_price, strike, put.bid_price, put.ask_price
            );
        }
    }
    DXEndpoint::get_instance()?.close_and_await_termination()
}

/// The calls and the puts of the strikes around a price.
fn options_around<'a>(
    series: &OptionSeries<&'a InstrumentProfile>,
    n: usize,
    price: f64,
) -> impl Iterator<Item = &'a InstrumentProfile> {
    series
        .get_n_strikes_around(n, price)
        .iter()
        .flat_map(|&strike| [series.call(strike), series.put(strike)])
        .flatten()
        .copied()
}

/// The ADDRESS argument; without it, `dxfeed.address` of the `dxfeed.system.properties` file or of the
/// `DXFEED_dxfeed.address` environment variable, and the demo feed as the last resort.
fn address_or_default(address: Option<String>) -> dxfeed_graal::Result<String> {
    match address {
        Some(address) => Ok(address),
        None => Ok(system::get_property("dxfeed.address")?.unwrap_or_else(|| "demo.dxfeed.com:7300".into())),
    }
}
