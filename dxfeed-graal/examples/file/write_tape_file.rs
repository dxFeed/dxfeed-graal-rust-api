//! Publishes two quotes into the text tape file `WriteTapeFile.out.txt` in the current directory, like
//! `WriteTapeFileSample` of the other dxFeed APIs. The `file_parser` sample reads such a file.
//!
//! Usage: `cargo run --example write_tape_file`.

use std::process::ExitCode;

use dxfeed_graal::event::Quote;
use dxfeed_graal::{DXEndpoint, Event, Role};

const FILE: &str = "WriteTapeFile.out.txt";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> dxfeed_graal::Result<()> {
    // Without a subscription a publisher drops its events; the wildcard subscribes the tape to every symbol.
    let endpoint = DXEndpoint::new_builder()
        .with_property("dxfeed.wildcard.enable", "true")
        .with_role(Role::Publisher)
        .build()?;
    endpoint.connect(&format!("tape:{FILE}[format=text]"))?;

    let quotes: [Event; 2] = [
        Quote {
            bid_price: 10.1,
            ask_price: 10.2,
            ..Quote::new("TEST1")
        }
        .into(),
        Quote {
            bid_price: 17.1,
            ask_price: 17.2,
            ..Quote::new("TEST2")
        }
        .into(),
    ];
    endpoint.get_publisher().publish_events(&quotes)?;

    // The events are written once they are processed, so wait before closing.
    endpoint.await_processed()?;
    endpoint.close_and_await_termination()?;
    println!("Wrote {} quotes to {FILE}", quotes.len());
    Ok(())
}
