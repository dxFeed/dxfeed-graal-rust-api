//! Reads the events of the given types and symbols from a tape file as fast as possible and prints them, like
//! `FileParserSample` of the other dxFeed APIs. The `write_tape_file` sample writes such a file.
//!
//! Usage: `cargo run --example FileParserSample -- <FILE> <TYPES> <SYMBOLS>`, with comma-separated event types and symbols,
//! for example `WriteTapeFile.out.txt Quote TEST1,TEST2`.

use std::process::ExitCode;

use dxfeed_graal::{DXEndpoint, EventType, Role};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [file, types, symbols] = args.as_slice() else {
        eprintln!("Usage: file_parser <file> <types> <symbols>");
        eprintln!("  file    - the tape file to read");
        eprintln!("  types   - comma-separated event types, for example Quote,Trade");
        eprintln!("  symbols - comma-separated symbols, for example IBM,AAPL");
        return ExitCode::from(2);
    };
    match run(file, types, symbols) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(file: &str, types: &str, symbols: &str) -> dxfeed_graal::Result<()> {
    let types = types
        .split(',')
        .map(str::parse)
        .collect::<dxfeed_graal::Result<Vec<EventType>>>()?;

    // A stream feed delivers every event of the file, not only the last one of each symbol.
    let endpoint = DXEndpoint::create_with_role(Role::StreamFeed)?;
    let subscription = endpoint.get_feed().create_subscription_with_types(&types)?;
    let mut counter = 0u64;
    subscription.add_event_listener(move |events| {
        for event in events {
            counter += 1;
            println!("{counter}: {event}");
        }
    })?;
    subscription.add_symbols(symbols.split(','))?;

    endpoint.connect(&format!("file:{file}[speed=max]"))?;
    // The endpoint disconnects when the file is read; closing waits for the listener to print the rest.
    endpoint.await_not_connected()?;
    endpoint.close_and_await_termination()
}
