//! Fetches the daily candles of the last 20 days of a symbol and prints them, like `FetchDailyCandlesSample` of the
//! other dxFeed APIs. It uses the default feed, whose address is the `dxfeed.address` system property.
//!
//! Usage: `cargo run --example FetchDailyCandlesSample -- <BASE_SYMBOL> [ADDRESS]`, for example `AAPL`. Without ADDRESS
//! the address is `dxfeed.address` of the `dxfeed.system.properties` file in the current directory or of the
//! `DXFEED_dxfeed.address` environment variable, else `demo.dxfeed.com:7300`.

use std::process::ExitCode;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dxfeed_graal::event::{Candle, CandlePeriod, CandleSymbol};
use dxfeed_graal::{DXEndpoint, DXFeed, system};

const DAYS: i64 = 20;
const DAY: i64 = 24 * 60 * 60 * 1000;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(base_symbol) = args.next() else {
        eprintln!("Usage: fetch_daily_candles <baseSymbol> [<address>]");
        eprintln!("  baseSymbol - the security symbol, for example IBM, AAPL or SPX");
        eprintln!("  address    - the address to connect to; dxfeed.address of dxfeed.system.properties by default");
        return ExitCode::from(2);
    };
    match run(&base_symbol, args.next()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(base_symbol: &str, address: Option<String>) -> dxfeed_graal::Result<()> {
    // The default feed connects to `dxfeed.address` when it is first used.
    system::set_property("dxfeed.address", &address_or_default(address)?)?;
    let candle_symbol = CandleSymbol::value_of_with(base_symbol, [CandlePeriod::DAY])?;
    let to_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64);
    let from_time = to_time - DAYS * DAY;
    println!("Fetching last {DAYS} days of candles for {base_symbol}");

    let promise = DXFeed::get_instance()?.get_time_series_promise::<Candle>(&candle_symbol, from_time, to_time)?;
    for candle in promise.wait(Duration::from_secs(5))? {
        println!("{candle}");
    }
    DXEndpoint::get_instance()?.close_and_await_termination()
}

/// The ADDRESS argument; without it, `dxfeed.address` of the `dxfeed.system.properties` file or of the
/// `DXFEED_dxfeed.address` environment variable, and the demo feed as the last resort.
fn address_or_default(address: Option<String>) -> dxfeed_graal::Result<String> {
    match address {
        Some(address) => Ok(address),
        None => Ok(system::get_property("dxfeed.address")?.unwrap_or_else(|| "demo.dxfeed.com:7300".into())),
    }
}
