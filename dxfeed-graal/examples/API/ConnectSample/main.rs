//! Usage: `cargo run --example ConnectSample -- <address> <types> <symbols> [<time>]`

use dxfeed_graal::{DXEndpoint, EventType, Symbol, TimeFormat, system};

fn print_usage() {
    let usage_string = format!(
        r#"
Usage:
ConnectSample <address> <types> <symbols> [<time>]

Where:
    address - The address to connect to retrieve data (remote host or local tape file).
              To pass an authorization token, add to the address: "[login=entitle:<token>]",
              e.g.: demo.dxfeed.com:7300[login=entitle:<token>]
    types   - Is comma-separated list of dxfeed event types ({names} or {class_names}).
    symbols - Is comma-separated list of symbol names to get events for (e.g. "IBM,AAPL,MSFT").
              for Candle event specify symbol with aggregation like in "AAPL{{=d}}"
    time    - Is from-time for history subscription in standard formats.
              Some examples of valid from-time:
                  20070101-123456
                  20070101-123456.123
                  2005-12-31 21:00:00
                  2005-12-31 21:00:00.123+03:00
                  2005-12-31 21:00:00.123+0400
                  2007-11-02Z
                  123456789 - value-in-milliseconds

Examples:
    ConnectSample demo.dxfeed.com:7300 Quote,Trade MSFT,IBM
    ConnectSample demo.dxfeed.com:7300 TimeAndSale AAPL
    ConnectSample demo.dxfeed.com:7300 Candle AAPL{{=d}} 20230901Z"#,
        names = get_event_type_enum_names_list(),
        class_names = get_event_type_enum_class_names_list(),
    );

    println!("{usage_string}");
}

fn main() -> dxfeed_graal::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 4 {
        print_usage();

        return Ok(());
    }

    system::set_property("monitoring.stat", "5s")?;
    system::set_property("log.file", "dx_trace.log")?;
    system::set_property("err.file", "dx_trace.err")?;

    // Parse args.
    let address = &args[1];
    let (parsed_types, _unknown_types) = parse_types(&args[2]);
    let symbols = parse_symbols(&args[3]);
    let mut time = -1_i64;

    if args.len() >= 5 {
        time = TimeFormat::DEFAULT.with_millis().with_time_zone().parse(&args[4])?;
    }

    // Create an endpoint and connect to a specified address.
    // The endpoint is closed at the end of the scope, when it is dropped.
    let endpoint = DXEndpoint::create()?;
    endpoint.connect(address)?;

    // Create a subscription with specified types attached to the feed.
    // The subscription is closed at the end of the scope before the endpoint, and close() waits for the listener calls
    // in progress.
    let sub = endpoint.get_feed().create_subscription_with_types(&parsed_types)?;

    // Add an event listener.
    sub.add_event_listener(|events| {
        for e in events {
            println!("{e}");
        }
    })?;

    // Add symbols.
    if time != -1 {
        sub.add_symbols(symbols.into_iter().map(|s| Symbol::time_series(s, time)))?;
    } else {
        sub.add_symbols(symbols)?;
    }

    let _ = std::io::stdin().read_line(&mut String::new());
    Ok(())
}

/// The names of all event types (QUOTE, TIME_AND_SALE etc.) in alphabetical order, comma-separated.
fn get_event_type_enum_names_list() -> String {
    let mut names: Vec<String> = EventType::ALL
        .iter()
        .map(|t| to_upper_snake_case(t.get_name()))
        .collect();
    names.sort();

    names.join(",")
}

/// The class names of all event types (Candle, Quote, TimeAndSale etc.) in alphabetical order, comma-separated.
fn get_event_type_enum_class_names_list() -> String {
    let mut names: Vec<&str> = EventType::ALL.iter().map(|t| t.get_name()).collect();
    names.sort();

    names.join(",")
}

/// `TimeAndSale` -> `TIME_AND_SALE`.
fn to_upper_snake_case(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut result = String::new();

    for (i, &c) in chars.iter().enumerate() {
        // A word starts at an uppercase letter that follows a lowercase one, or that ends a run of uppercase letters.
        if i > 0
            && c.is_ascii_uppercase()
            && (chars[i - 1].is_ascii_lowercase() || chars.get(i + 1).is_some_and(|n| n.is_ascii_lowercase()))
        {
            result.push('_');
        }

        result.push(c.to_ascii_uppercase());
    }

    result
}

/// The event types of a comma-separated list of names (QUOTE) or class names (Quote), without repeats, and the unknown
/// names; `all`, `All`, `ALL` or `*` for all the types.
fn parse_types(types: &str) -> (Vec<EventType>, Vec<String>) {
    let types = types.trim();

    if types.is_empty() {
        return Default::default();
    }

    if matches!(types, "*" | "all" | "All" | "ALL") {
        return (EventType::ALL.to_vec(), Vec::new());
    }

    let (mut result, mut unknown) = (Vec::new(), Vec::new());

    for t in types.split(',').map(str::trim).filter(|t| !t.is_empty()) {
        let found = EventType::ALL
            .iter()
            .copied()
            .find(|e| to_upper_snake_case(e.get_name()) == t.to_uppercase() || e.get_name() == t);

        match found {
            Some(event_type) if !result.contains(&event_type) => result.push(event_type),
            Some(_) => {}
            None => unknown.push(t.to_owned()),
        }
    }

    (result, unknown)
}

/// The symbols of a comma-separated list, where a comma inside braces, parentheses or brackets does not separate
/// symbols, as in `AAPL{=d,price=bid}`; `all` or `*` for all the symbols.
fn parse_symbols(symbols: &str) -> Vec<Symbol> {
    let mut parsed: Vec<&str> = Vec::new();
    let (mut depth, mut start) = (0i32, 0);

    for (i, c) in symbols.char_indices() {
        match c {
            '{' | '(' | '[' => depth += 1,
            '}' | ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                parsed.push(&symbols[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }

    parsed.push(&symbols[start..]);

    let parsed: Vec<&str> = parsed.into_iter().map(str::trim).filter(|s| !s.is_empty()).collect();

    if parsed.iter().any(|s| *s == "*" || *s == "all") {
        return vec![Symbol::Wildcard];
    }

    parsed.into_iter().map(Symbol::from).collect()
}
