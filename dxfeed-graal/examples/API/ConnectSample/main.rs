//! Connects to an address, subscribes to the events of the given types and symbols and prints them until Enter is
//! pressed, like `ConnectSample` of the other dxFeed APIs. With a time, it subscribes to the time series of the
//! symbols from that time on.
//!
//! Usage: `cargo run --example ConnectSample -- <ADDRESS> <TYPES> <SYMBOLS> [TIME]`, for example
//! `demo.dxfeed.com:7300 Quote,Trade AAPL,IBM` or `demo.dxfeed.com:7300 Candle "AAPL{=d}" 20260101-000000`.

use std::process::ExitCode;

use dxfeed_graal::{DXEndpoint, EventType, Symbol, TimeFormat};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (address, types, symbols, time) = match args.as_slice() {
        [address, types, symbols] => (address, types, symbols, None),
        [address, types, symbols, time] => (address, types, symbols, Some(time)),
        _ => {
            eprintln!("Usage: connect <address> <types> <symbols> [<time>]");
            eprintln!("  address - the address to connect to, for example demo.dxfeed.com:7300");
            eprintln!("  types   - comma-separated event types, for example Quote,Trade; all or * for all the types");
            eprintln!("  symbols - comma-separated symbols, for example IBM,AAPL{{=d}}; all or * for all the symbols");
            eprintln!("  time    - the time of the first events of a time series, for example 20260101-000000,");
            eprintln!("            2026-01-01 00:00:00+03:00 or milliseconds since the epoch");
            return ExitCode::from(2);
        }
    };
    match run(address, types, symbols, time.map(String::as_str)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(address: &str, types: &str, symbols: &str, time: Option<&str>) -> dxfeed_graal::Result<()> {
    let types = parse_types(types)?;
    let mut symbols = parse_symbols(symbols);
    // With a time, each symbol becomes the time series of the symbol from that time on.
    if let Some(time) = time {
        let from_time = TimeFormat::DEFAULT.parse(time)?;
        symbols = symbols
            .into_iter()
            .map(|symbol| Symbol::time_series(symbol, from_time))
            .collect();
    }

    // The wildcard symbol needs `dxfeed.wildcard.enable`.
    let endpoint = DXEndpoint::new_builder()
        .with_property("dxfeed.wildcard.enable", "true")
        .build()?;
    let subscription = endpoint.get_feed().create_subscription_with_types(&types)?;
    // Listeners go before symbols, so the first events are not missed.
    subscription.add_event_listener(|events| {
        for event in events {
            println!("{event}");
        }
    })?;
    subscription.add_symbols(&symbols)?;
    endpoint.connect(address)?;
    eprintln!("Press Enter to stop.");

    // Enter or the end of input stops the program.
    let _ = std::io::stdin().read_line(&mut String::new());
    endpoint.close_and_await_termination()
}

/// The event types of comma-separated names in any case; `all` or `*` for all the types.
fn parse_types(types: &str) -> dxfeed_graal::Result<Vec<EventType>> {
    if types == "all" || types == "*" {
        return Ok(EventType::ALL.to_vec());
    }
    types
        .split(',')
        .map(|name| {
            EventType::ALL
                .iter()
                .copied()
                .find(|t| t.name().eq_ignore_ascii_case(name.trim()))
                .ok_or_else(|| dxfeed_graal::Error::InvalidArgument(format!("unknown event type {name:?}")))
        })
        .collect()
}

/// The symbols of a comma-separated list, where a comma inside braces, parentheses or brackets does not separate
/// symbols, as in `AAPL{=d,price=bid}`; `all` or `*` for all the symbols.
fn parse_symbols(symbols: &str) -> Vec<Symbol> {
    if symbols == "all" || symbols == "*" {
        return vec![Symbol::Wildcard];
    }
    let mut result = Vec::new();
    let (mut depth, mut start) = (0i32, 0);
    for (i, c) in symbols.char_indices() {
        match c {
            '{' | '(' | '[' => depth += 1,
            '}' | ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                result.push(symbols[start..i].into());
                start = i + 1;
            }
            _ => {}
        }
    }
    result.push(symbols[start..].into());
    result
}
