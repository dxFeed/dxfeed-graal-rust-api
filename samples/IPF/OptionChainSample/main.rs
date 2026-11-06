//! The properties file is read from the current directory, so run the sample from its directory:
//! `cargo run --bin OptionChainSample -- <ipf-file> <symbol> <strikes-number> <months-number>`.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use dxfeed_graal::event::{Quote, Trade};
use dxfeed_graal::ipf::InstrumentProfileReader;
use dxfeed_graal::ipf::option::OptionChainsBuilder;
use dxfeed_graal::{DXFeed, Error, Promise};

fn main() -> dxfeed_graal::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 5 {
        print!(
            r#"
Usage:
OptionChainSample <ipf-file> <symbol> <strikes-number> <months-number>

Where:
    <ipf-file>       - a name of instrument profiles file
    <symbol>         - the product or underlying symbol
    <strikes-number> - a number of strikes to print for each series
    <months-number>  - a number of months to print
Examples:
    OptionChainSample "https://demo:demo@tools.dxfeed.com/ipf" AAPL 5 5
"#
        );

        return Ok(());
    }

    let ipf_file = &args[1];
    // Specified instrument name, for example AAPL, IBM, MSFT, etc.
    let symbol = &args[2];
    let strikes_number: usize = args[3]
        .parse()
        .map_err(|e| Error::InvalidArgument(format!("{}: {e}", args[3])))?;
    let mut months_number: usize = args[4]
        .parse()
        .map_err(|e| Error::InvalidArgument(format!("{}: {e}", args[4])))?;

    let feed = DXFeed::get_instance()?;

    // subscribe to trade to learn an instrument last price
    println!("Waiting for price of {symbol} ...");
    let trade = feed
        .get_last_event_promise::<Trade>(symbol)?
        .wait(Duration::from_secs(1))?;

    let price = trade.price;

    println!("Price of {symbol} is {price:?}");
    println!("Reading instruments from {ipf_file}");

    let instruments = InstrumentProfileReader::new()?.read_from_file(ipf_file)?;

    println!("Building option chains ...");
    let chains = OptionChainsBuilder::build(&instruments).into_chains();
    let chain = chains
        .get(symbol)
        .ok_or_else(|| Error::InvalidArgument(format!("no option chain of {symbol}")))?;

    months_number = months_number.min(chain.get_series().len());

    let series_vector = &chain.get_series()[..months_number];

    println!("Requesting option quotes ...");

    let mut quotes: HashMap<String, Promise<Quote>> = HashMap::new();

    for series in series_vector {
        for &strike in series.get_n_strikes_around(strikes_number, price) {
            if let Some(call) = series.call(strike) {
                quotes.insert(call.symbol.clone(), feed.get_last_event_promise::<Quote>(&call.symbol)?);
            }

            if let Some(put) = series.put(strike) {
                quotes.insert(put.symbol.clone(), feed.get_last_event_promise::<Quote>(&put.symbol)?);
            }
        }
    }

    // ignore timeout and continue to print retrieved quotes even on timeout
    let deadline = Instant::now() + Duration::from_secs(10);

    for promise in quotes.values() {
        let _ = promise.wait(deadline.saturating_duration_since(Instant::now()));
    }

    println!("Printing option series ...");

    for series in series_vector {
        println!("Option series {series}");
        println!(
            "    {:>10} {:>10} {:>10} {:>10} {:>10}",
            "C.BID", "C.ASK", "STRIKE", "P.BID", "P.ASK"
        );

        for &strike in series.get_n_strikes_around(strikes_number, price) {
            let mut call_quote = Quote::new("");
            let mut put_quote = Quote::new("");

            if let Some(call) = series.call(strike)
                && let Ok(Some(new_call_quote)) = quotes[&call.symbol].get_result()
            {
                call_quote = new_call_quote;
            }

            if let Some(put) = series.put(strike)
                && let Ok(Some(new_put_quote)) = quotes[&put.symbol].get_result()
            {
                put_quote = new_put_quote;
            }

            println!(
                "    {:10.3} {:10.3} {:10.3} {:10.3} {:10.3}",
                call_quote.bid_price, call_quote.ask_price, strike, put_quote.bid_price, put_quote.ask_price
            );
        }
    }

    Ok(())
}
