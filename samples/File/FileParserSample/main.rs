//! Reads the events of the given types and symbols from a tape file as fast as possible and prints them.
//!
//! Usage: `cargo run --bin FileParserSample -- <file> <types> <symbols>`

use dxfeed_graal::{DXEndpoint, EventType, Role, Symbol};

fn print_usage() {
    let usage_string = format!(
        r#"
Usage:
FileParserSample <file> <type> <symbol>

Where:
    file    - Is a file name.
    types   - Is comma-separated list of dxfeed event types ({names} or {class_names}).
    symbols - Is comma-separated list of symbol names to get events for (e.g. "IBM,AAPL,MSFT")."#,
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

    let mut event_counter: usize = 0;

    // Parse args.
    let file_name = &args[1];
    let (parsed_types, _unknown_types) = parse_types(&args[2]);
    let symbols = parse_symbols(&args[3]);

    // Create an endpoint specifically for file parsing.
    let endpoint = DXEndpoint::create_with_role(Role::StreamFeed)?;
    let feed = endpoint.get_feed();

    // Subscribe to a specified event and symbol.
    let sub = feed.create_subscription_with_types(&parsed_types)?;
    sub.add_event_listener(move |events| {
        for e in events {
            event_counter += 1;
            println!("{event_counter}: {e}");
        }
    })?;

    // Add symbols.
    sub.add_symbols(&symbols)?;

    // Connect an endpoint to a file.
    endpoint.connect(&format!("file:{file_name}[speed=max]"))?;

    // Wait until a file is completely parsed.
    endpoint.await_not_connected()?;

    // Close the endpoint when we're done.
    // This method will gracefully close the endpoint, waiting while data processing completes.
    endpoint.close_and_await_termination()?;

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
