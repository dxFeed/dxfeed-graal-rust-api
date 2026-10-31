//! Gets the hourly candles and the time and sales of a symbol for a period from the CandleData service with a
//! `HistoryEndpoint`, like `CandleDataSample` of the other dxFeed APIs.
//!
//! Usage: `cargo run --example CandleDataSample -- <ADDRESS> <USER> <PASSWORD> <BASE_SYMBOL> <FROM> <TO>`; run it without
//! arguments for the details.

use std::process::ExitCode;

use dxfeed_graal::TimeFormat;
use dxfeed_graal::event::{Candle, CandlePeriod, CandleSymbol, CandleType, TimeAndSale};
use dxfeed_graal::history::HistoryEndpoint;

const USAGE: &str = r#"
Usage:
candle_data <address> <user> <password> <baseSymbol> <fromTime> <toTime>

Where:
    address    - The address to connect to retrieve data.
                 Examples:
                     https://tools-demo.dxfeed.com/candledata-preview
                     https://tools.dxfeed.com/candledata
    user       - User login name.
    password   - User password.
    baseSymbol - Base symbol name for Candle or TnS to get events for (e.g. "IBM", "AAPL", "MSFT").
    fromTime   - Is from-time for history subscription in standard formats.
                 Some examples of valid from-time:
                     20070101-123456
                     20070101-123456.123
                     2005-12-31 21:00:00
                     2005-12-31 21:00:00.123+03:00
                     2005-12-31 21:00:00.123+0400
                     2007-11-02Z
                     123456789 - value-in-milliseconds
    toTime     - Is to-time for history subscription in standard formats.

Examples:
    candle_data https://tools-demo.dxfeed.com/candledata-preview demo demo IBM 20250818-120000 20250819-130000"#;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 6 {
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
    let (address, user, password, base_symbol) = (&args[0], &args[1], &args[2], &args[3]);
    let time_format = TimeFormat::DEFAULT.with_millis().with_time_zone();
    let from_time = time_format.parse(&args[4])?;
    let to_time = time_format.parse(&args[5])?;

    let builder = HistoryEndpoint::new_builder()
        .with_address(address)
        .with_user_name(user)
        .with_password(password);
    let endpoint = builder.build()?;
    // The hourly candles: {=h}.
    let candle_symbol = CandleSymbol::value_of_with(base_symbol, [CandlePeriod::value_of(1.0, CandleType::Hour)])?;
    let candles = endpoint.get_time_series::<Candle>(candle_symbol, from_time, to_time)?;
    println!("Received candle count: {}", candles.len());

    let time_and_sales = builder
        .build()?
        .get_time_series::<TimeAndSale>(base_symbol.as_str(), from_time, to_time)?;
    println!("Received tns count: {}", time_and_sales.len());
    Ok(())
}
