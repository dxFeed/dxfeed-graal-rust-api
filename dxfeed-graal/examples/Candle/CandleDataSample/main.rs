//! Gets the hourly candles and the time and sales of a symbol for a period from the CandleData service with a
//! `HistoryEndpoint`.
//!
//! Usage: `cargo run --example CandleDataSample -- <address> <user> <password> <baseSymbol> <fromTime> <toTime>`

use dxfeed_graal::event::{Candle, CandlePeriod, CandleSymbol, CandleType, TimeAndSale};
use dxfeed_graal::history::HistoryEndpoint;
use dxfeed_graal::{TimeFormat, system};

fn print_usage() {
    let usage_string = r#"
Usage:
CandleDataSample <address> <user> <password> <baseSymbol> <fromTime> <toTime>

Where:
    address    - The address to connect to retrieve data.
                 Examples:
                     https://tools-demo.dxfeed.com/candledata-preview
                     https://tools.dxfeed.com/candledata
    user       - User login name.
    password   - User password.
    baseSymbol - Base symbol name for Candle or TnS to get events for (e.g. "IBM", "AAPL", "MSFT").
    fromTime   - Is from-time for history subscription in standard formats.
                 Same examples of valid from-time:
                     20070101-123456
                     20070101-123456.123
                     2005-12-31 21:00:00
                     2005-12-31 21:00:00.123+03:00
                     2005-12-31 21:00:00.123+0400
                     2007-11-02Z
                     123456789 - value-in-milliseconds
    toTime     - Is to-time for history subscription in standard formats.

Examples:
    CandleDataSample https://tools-demo.dxfeed.com/candledata-preview demo demo IBM 20250818-120000 20250819-130000"#;

    println!("{usage_string}");
}

fn main() -> dxfeed_graal::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 7 {
        print_usage();

        return Ok(());
    }

    // Disable QD logging.
    system::set_property("log.className", "com.devexperts.logging.InterceptableLogging")?;
    system::set_property("log.level", "OFF")?;
    system::set_property("err.level", "OFF")?;

    // Parse args.
    let address = &args[1];
    let user = &args[2];
    let password = &args[3];
    let base_symbol = &args[4];
    let from_time = TimeFormat::DEFAULT.with_millis().with_time_zone().parse(&args[5])?;
    let to_time = TimeFormat::DEFAULT.with_millis().with_time_zone().parse(&args[6])?;

    let builder = HistoryEndpoint::new_builder()
        .with_address(address)
        .with_user_name(user)
        .with_password(password);
    let endpoint = builder.build()?;
    let res = endpoint.get_time_series::<Candle>(
        CandleSymbol::value_of_with(base_symbol, [CandlePeriod::value_of(1.0, CandleType::Hour)])?, // {=h}
        from_time,
        to_time,
    )?;

    println!("Received candle count: {}", res.len());
    let res_tns = builder
        .build()?
        .get_time_series::<TimeAndSale>(base_symbol.as_str(), from_time, to_time)?;

    println!("Received tns count: {}", res_tns.len());

    Ok(())
}
