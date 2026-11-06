//! Subscribes to the Candle events of a symbol from the specified time and prints them.
//!
//! Usage: `cargo run --bin CandleSample -- <address> <symbol> [<fromTime>] [<attribute>=<value> ...]`

use dxfeed_graal::event::{
    Candle, CandleAlignment, CandleAttribute, CandleExchange, CandlePeriod, CandlePrice, CandlePriceLevel,
    CandleSession, CandleSymbol, CandleType,
};
use dxfeed_graal::{DXEndpoint, Symbol, TimeFormat};

fn print_usage() {
    let usage_string = r#"
Usage:
CandleSample <address> <symbol> [<fromTime>] [<attribute>=<value> ...]

Where:
    address   - The address to connect to retrieve data (e.g. "demo.dxfeed.com:7300") or a path to a tape file.
    symbol    - The candle symbol: a base symbol (e.g. "AAPL") with optional attributes (e.g. "AAPL{=d}").
    fromTime  - The from-time of the history subscription in standard formats (e.g. "2025-12-31",
                "20251231-123456", "1735689600000" in milliseconds). The default is 1970-01-01.
    attribute - Adds an attribute to the symbol:
                    exchange   - The exchange code letter (e.g. "exchange=A").
                    period     - The number of aggregation period units (e.g. "period=5"). The default is 1.
                    type       - The aggregation period type: "t" (TICK), "s" (SECOND), "m" (MINUTE),
                                 "h" (HOUR), "d" (DAY), "w" (WEEK), "mo" (MONTH), "o" (OPTEXP), "y" (YEAR),
                                 "v" (VOLUME), "p" (PRICE), "pm" (PRICE_MOMENTUM), "pr" (PRICE_RENKO).
                                 The default is TICK.
                    price      - The price type: "last", "bid", "ask", "mark", "s" (SETTLEMENT).
                    session    - "true" for the regular trading hours only, "false" for any session.
                    alignment  - The alignment: "m" (MIDNIGHT), "s" (SESSION).
                    priceLevel - The price level (e.g. "priceLevel=0.5").

Examples:
    CandleSample demo.dxfeed.com:7300 AAPL{=d} 2025-12-01
    CandleSample demo.dxfeed.com:7300 AAPL 2025-12-01 period=5 type=m session=true
"#;

    println!("{usage_string}");
}

fn main() -> dxfeed_graal::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 3 {
        print_usage();

        return Ok(());
    }

    let address = &args[1];
    let base_symbol = &args[2];
    let mut from_time: i64 = 0;
    let mut attributes: Vec<CandleAttribute> = Vec::new();
    let mut period_value: Option<f64> = None;
    let mut period_type: Option<String> = None;

    for (i, arg) in args.iter().enumerate().skip(3) {
        let Some((name, value)) = arg.split_once('=') else {
            if i != 3 {
                eprintln!("Unexpected argument: {arg}");
                print_usage();

                std::process::exit(1);
            }

            from_time = TimeFormat::DEFAULT.parse(arg)?;

            continue;
        };

        if name == "exchange" && value.len() == 1 {
            attributes.push(CandleExchange::value_of(value.as_bytes()[0] as char).into());
        } else if name == "period" {
            period_value = Some(stod(value));
        } else if name == "type" {
            period_type = Some(value.to_owned());
        } else if name == "price" {
            attributes.push(CandlePrice::parse(value)?.into());
        } else if name == "session" {
            attributes.push(CandleSession::parse(value)?.into());
        } else if name == "alignment" {
            attributes.push(CandleAlignment::parse(value)?.into());
        } else if name == "priceLevel" {
            attributes.push(CandlePriceLevel::value_of(stod(value))?.into());
        } else {
            eprintln!("Invalid attribute: {arg}");
            print_usage();

            std::process::exit(1);
        }
    }

    if period_value.is_some() || period_type.is_some() {
        let candle_type = match &period_type {
            Some(period_type) => CandleType::parse(period_type)?,
            None => CandleType::Tick,
        };

        attributes.push(CandlePeriod::value_of(period_value.unwrap_or(1.0), candle_type).into());
    }

    let symbol = CandleSymbol::value_of_with(base_symbol, attributes)?;

    println!(
        "Subscribing to {symbol} from {}",
        TimeFormat::DEFAULT.format(from_time)?
    );

    let endpoint = DXEndpoint::create()?;
    endpoint.connect(address)?;
    let subscription = endpoint.get_feed().create_subscription::<Candle>()?;

    // Listener must be attached before symbols are added.
    subscription.add_event_listener(|candles| {
        for candle in candles {
            println!("{candle}");
        }
    })?;

    // The history subscription: the candles from the specified time and then the updates.
    subscription.add_symbols([Symbol::time_series(symbol, from_time)])?;

    println!("Press enter to stop");
    let _ = std::io::stdin().read_line(&mut String::new());

    endpoint.close_and_await_termination()?;

    Ok(())
}

/// Parses a number like `std::stod` of the C++ sample: a malformed number prints its error and exits with 1.
fn stod(value: &str) -> f64 {
    value.parse().unwrap_or_else(|e| {
        eprintln!("{e}");

        std::process::exit(1);
    })
}
