//! The Connect tool: subscribes to events at an address and prints them, as the Connect tool of the C++ API does.

use std::error::Error;

use dxfeed_graal::event::IndexedEventSource;
use dxfeed_graal::{DXEndpoint, Role, Symbol, TimeFormat, system};

use crate::args::{self, Arg, Parsed, optional, required};
use crate::help::Info;
use crate::tape;

pub const INFO: Info = Info {
    name: "Connect",
    short_description: "Connects to specified address(es).",
    description: "\nConnects to the specified address(es) and subscribes to the specified symbols.\n",
    usage: &["Connect <address> <types> <symbols> [<options>]"],
    additional_info: &[],
    args: &[
        required(Arg::Address),
        required(Arg::Types),
        required(Arg::Symbols),
        optional(Arg::FromTime),
        optional(Arg::Source),
        optional(Arg::Properties),
        optional(Arg::Tape),
        optional(Arg::Quiet),
        optional(Arg::ForceStream),
        optional(Arg::Help),
    ],
};

/// The symbols of the subscription: time-series symbols from `--from-time` if it is given, else indexed symbols of
/// `--source` if it is given, else the symbols as they are.
fn subscription_symbols(args: &Parsed) -> Result<Vec<Symbol>, Box<dyn Error>> {
    let symbols = args::symbols(args.get(Arg::Symbols))?;
    if let Some(from_time) = args.get(Arg::FromTime) {
        let from_time = TimeFormat::DEFAULT.parse(from_time)?;
        return Ok(symbols
            .into_iter()
            .map(|symbol| Symbol::time_series(symbol, from_time))
            .collect());
    }
    if let Some(source) = args.get(Arg::Source) {
        let source = IndexedEventSource::value_of(source)?;
        return Ok(symbols
            .into_iter()
            .map(|symbol| Symbol::indexed(symbol, source))
            .collect());
    }
    Ok(symbols)
}

/// Connects and prints the events until the process is stopped.
pub fn run(args: &Parsed) -> Result<(), Box<dyn Error>> {
    let properties = args::parse_properties(args.get(Arg::Properties));
    for (key, value) in &properties {
        system::set_property(key, value)?;
    }
    let types = args::event_types(args.get(Arg::Types))?;
    let symbols = subscription_symbols(args)?;

    let role = if args.has(Arg::ForceStream) {
        Role::StreamFeed
    } else {
        Role::Feed
    };
    let endpoint = DXEndpoint::new_builder()
        .with_role(role)
        .with_properties(properties.iter().map(|(key, value)| (key, value)))
        .with_name(&format!("{}Tool-Feed", INFO.name))
        .build()?;
    let subscription = endpoint.get_feed().create_subscription_with_types(&types)?;
    if !args.has(Arg::Quiet) {
        tape::print_events(&subscription)?;
    }
    let _tape = match args.get(Arg::Tape) {
        Some(file) => Some(tape::tape(INFO.name, file, &properties, &subscription)?),
        None => None,
    };
    subscription.add_symbols(symbols)?;
    endpoint.connect(args.get(Arg::Address).unwrap_or_default())?;
    loop {
        std::thread::park();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args::{ParseResult, parse};

    fn parsed(args: &[&str]) -> Parsed {
        let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
        match parse(INFO.args, &args) {
            ParseResult::Ok(parsed) => parsed,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn symbols_as_they_are() {
        // The symbol list parser of the SDK returns a set, in no particular order.
        let symbols = subscription_symbols(&parsed(&["file", "Quote", "AAPL,IBM"])).unwrap();
        assert_eq!(symbols.len(), 2, "{symbols:?}");
        assert!(symbols.contains(&Symbol::from("AAPL")) && symbols.contains(&Symbol::from("IBM")));
    }

    #[test]
    fn time_series_symbols_from_a_time() {
        let symbols = subscription_symbols(&parsed(&["file", "Candle", "AAPL{=d}", "-f", "20260101-000000Z"])).unwrap();
        let from_time = TimeFormat::DEFAULT.parse("20260101-000000Z").unwrap();
        assert_eq!(symbols, [Symbol::time_series("AAPL{=d}", from_time)]);
    }

    #[test]
    fn indexed_symbols_of_a_source() {
        let symbols = subscription_symbols(&parsed(&["file", "Order", "AAPL", "--source", "NTV"])).unwrap();
        let source = IndexedEventSource::value_of("NTV").unwrap();
        assert_eq!(symbols, [Symbol::indexed("AAPL", source)]);
    }

    #[test]
    fn the_time_wins_over_the_source() {
        let symbols = subscription_symbols(&parsed(&["file", "Order", "AAPL", "-s", "NTV", "-f", "0"])).unwrap();
        assert_eq!(symbols, [Symbol::time_series("AAPL", 0)]);
    }

    #[test]
    fn a_wrong_time_or_source_is_an_error() {
        assert!(subscription_symbols(&parsed(&["file", "Candle", "AAPL", "-f", "not a time"])).is_err());
        assert!(subscription_symbols(&parsed(&["file", "Order", "AAPL", "-s", "TOO_LONG"])).is_err());
    }
}
