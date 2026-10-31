//! Fetches last 20 days of candles for a specified symbol, prints them, and exits.
//!
//! The properties file is read from the current directory, so run the sample from its directory:
//! `cargo run --example FetchDailyCandlesSample -- <baseSymbol>`.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dxfeed_graal::DXFeed;
use dxfeed_graal::event::{Candle, CandlePeriod, CandleSymbol};

fn fetch_and_print(candle_symbol: &CandleSymbol, to_time: i64, from_time: i64) -> dxfeed_graal::Result<()> {
    // Use the default DXFeed instance for that data feed address is defined by "dxfeed.system.properties" file
    let result = DXFeed::get_instance()?
        .get_time_series_promise::<Candle>(candle_symbol, from_time, to_time)?
        .wait(Duration::from_secs(5))?;

    for candle in &result {
        println!("{candle}");
    }

    Ok(())
}

fn main() -> dxfeed_graal::Result<()> {
    const DAYS: u64 = 20;

    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        print!(
            r#"
Usage:
FetchDailyCandlesSample <baseSymbol>

Where:
    baseSymbol - Is security symbol (e.g. IBM, AAPL, SPX etc.)
"#
        );

        return Ok(());
    }

    // Specified instrument name, for example AAPL, IBM, MSFT, etc.
    let base_symbol = &args[1];
    let candle_symbol = CandleSymbol::value_of_with(base_symbol, [CandlePeriod::DAY])?;
    let to_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64);
    let from_time = to_time - Duration::from_secs(DAYS * 24 * 60 * 60).as_millis() as i64;

    println!("Fetching last {DAYS} days of candles for {base_symbol}");

    fetch_and_print(&candle_symbol, to_time, from_time)
}
