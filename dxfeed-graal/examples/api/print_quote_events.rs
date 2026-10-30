//! Prints the quotes of a symbol until Enter is pressed, like `PrintQuoteEventsSample` of the other dxFeed APIs. It
//! uses the default feed, whose address is the `dxfeed.address` system property.
//!
//! Usage: `cargo run --example print_quote_events -- [SYMBOL] [ADDRESS]`, by default `AAPL`. Without ADDRESS the
//! address is `dxfeed.address` of the `dxfeed.system.properties` file in the current directory or of the
//! `DXFEED_dxfeed.address` environment variable, else `demo.dxfeed.com:7300`.

use std::process::ExitCode;

use dxfeed_graal::event::Quote;
use dxfeed_graal::{DXEndpoint, DXFeed, system};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let symbol = args.next().unwrap_or_else(|| "AAPL".into());
    match run(&symbol, args.next()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(symbol: &str, address: Option<String>) -> dxfeed_graal::Result<()> {
    let address = address_or_default(address)?;
    // The default feed connects to `dxfeed.address` when it is first used.
    system::set_property("dxfeed.address", &address)?;
    let subscription = DXFeed::get_instance()?.create_subscription::<Quote>()?;
    // Listeners go before symbols, so the first quotes are not missed.
    subscription.add_event_listener(|quotes| {
        for quote in quotes {
            println!("{quote}");
        }
    })?;
    subscription.add_symbols([symbol])?;
    eprintln!("Printing quotes of {symbol} from {address}; press Enter to stop.");

    // Enter or the end of input stops the program.
    let _ = std::io::stdin().read_line(&mut String::new());
    DXEndpoint::get_instance()?.close_and_await_termination()
}

/// The ADDRESS argument; without it, `dxfeed.address` of the `dxfeed.system.properties` file or of the
/// `DXFEED_dxfeed.address` environment variable, and the demo feed as the last resort.
fn address_or_default(address: Option<String>) -> dxfeed_graal::Result<String> {
    match address {
        Some(address) => Ok(address),
        None => Ok(system::get_property("dxfeed.address")?.unwrap_or_else(|| "demo.dxfeed.com:7300".into())),
    }
}
