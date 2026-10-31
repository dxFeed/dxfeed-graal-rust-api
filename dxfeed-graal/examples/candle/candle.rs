//! Subscribes to the candles of a symbol from a time on, with the given candle attributes, and prints them until Enter
//! is pressed, like `CandleSample` of the other dxFeed APIs.
//!
//! Usage: `cargo run --example candle -- <ADDRESS> <SYMBOL> [FROM_TIME] [ATTRIBUTE=VALUE ...]`, for example
//! `demo.dxfeed.com:7300 AAPL 20260101-000000 period=1 type=day price=mark`.

use std::process::ExitCode;

use dxfeed_graal::event::{
    Candle, CandleAlignment, CandleAttribute, CandlePeriod, CandlePrice, CandleSession, CandleSymbol, CandleType,
};
use dxfeed_graal::{DXEndpoint, Error, Symbol, TimeFormat};

const USAGE: &str = "Usage: candle <address> <symbol> [<fromTime>] [<attribute>=<value> ...]
  address   - the address to connect to, for example demo.dxfeed.com:7300
  symbol    - the base symbol of the candles, for example AAPL
  fromTime  - the time of the first candle, for example 20260101-000000, 2026-01-01 00:00:00+03:00 or
              milliseconds since the epoch; 0 by default
  attribute - an attribute of the candle symbol:
              exchange   - the exchange code, a character, for example Q
              period     - the number of units of the period, for example 5
              type       - the unit of the period: tick, second, minute, hour, day, week, month, optexp, year,
                           volume, price, price_momentum, price_renko or their short names (t, s, m, h, d, ...)
              price      - the price of the candles: last, bid, ask, mark or settlement
              session    - the trading session: any or regular
              alignment  - the alignment of the candles: midnight or session
              priceLevel - the price level of the price candles, for example 0.5";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [address, symbol, rest @ ..] = args.as_slice() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    match run(address, symbol, rest) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Error::InvalidArgument(message)) => {
            eprintln!("{message}\n{USAGE}");
            ExitCode::from(2)
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(address: &str, base_symbol: &str, args: &[String]) -> dxfeed_graal::Result<()> {
    let mut from_time = 0;
    let mut attributes = Vec::new();
    let (mut period_value, mut period_type) = (None, None);
    for (i, arg) in args.iter().enumerate() {
        let Some((name, value)) = arg.split_once('=') else {
            if i != 0 {
                return Err(Error::InvalidArgument(format!("Unexpected argument: {arg}")));
            }
            from_time = TimeFormat::DEFAULT.parse(arg)?;
            continue;
        };
        let invalid = || Error::InvalidArgument(format!("Invalid attribute: {arg}"));
        let mut chars = value.chars();
        match (name, chars.next(), chars.next()) {
            ("exchange", Some(code), None) => attributes.push(CandleAttribute::Exchange(code)),
            ("period", ..) => period_value = Some(value.parse::<f64>().map_err(|_| invalid())?),
            ("type", ..) => period_type = Some(value.parse::<CandleType>()?),
            ("price", ..) => attributes.push(value.parse::<CandlePrice>()?.into()),
            ("session", ..) => attributes.push(value.parse::<CandleSession>()?.into()),
            ("alignment", ..) => attributes.push(value.parse::<CandleAlignment>()?.into()),
            ("priceLevel", ..) => attributes.push(CandleAttribute::PriceLevel(value.parse().map_err(|_| invalid())?)),
            _ => return Err(invalid()),
        }
    }
    if period_value.is_some() || period_type.is_some() {
        let period = CandlePeriod::value_of(period_value.unwrap_or(1.0), period_type.unwrap_or(CandleType::Tick));
        attributes.push(period.into());
    }
    let symbol = CandleSymbol::value_of_with(base_symbol, attributes)?;
    println!(
        "Subscribing to {symbol} from {}",
        TimeFormat::DEFAULT.format(from_time)?
    );

    let endpoint = DXEndpoint::create()?;
    endpoint.connect(address)?;
    let subscription = endpoint.feed().create_subscription::<Candle>()?;
    // Listeners go before symbols, so the first candles are not missed.
    subscription.add_event_listener(|candles| {
        for candle in candles {
            println!("{candle}");
        }
    })?;
    // The candles from the time on, and then their updates.
    subscription.add_symbols([Symbol::time_series(symbol, from_time)])?;
    println!("Press enter to stop");

    // Enter or the end of input stops the program.
    let _ = std::io::stdin().read_line(&mut String::new());
    endpoint.close_and_await_termination()
}
