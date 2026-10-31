//! Prints the state changes of an endpoint while it connects, disconnects and connects again, like `ReconnectSample`
//! of the other dxFeed APIs.
//!
//! Usage: `cargo run --example reconnect -- [SYMBOL] [ADDRESS]`, by default `ETH/USD:GDAX`. Without ADDRESS the
//! address is `dxfeed.address` of the `dxfeed.system.properties` file in the current directory or of the
//! `DXFEED_dxfeed.address` environment variable, else `demo.dxfeed.com:7300`.

use std::process::ExitCode;
use std::thread;
use std::time::Duration;

use dxfeed_graal::event::Quote;
use dxfeed_graal::{DXEndpoint, system};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let symbol = args.next().unwrap_or_else(|| "ETH/USD:GDAX".into());
    match run(&symbol, args.next()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(symbol: &str, address: Option<String>) -> dxfeed_graal::Result<()> {
    let address = &address_or_default(address)?;
    let endpoint = DXEndpoint::create()?;
    endpoint.add_state_listener(|old, new| println!("Connection state changed: {old}->{new}"))?;
    endpoint.connect(address)?;

    let subscription = endpoint.get_feed().create_subscription::<Quote>()?;
    subscription.add_event_listener(|quotes| {
        for quote in quotes {
            println!("{quote}");
        }
    })?;
    subscription.add_symbols([symbol])?;

    thread::sleep(Duration::from_secs(5));
    println!("Disconnecting");
    endpoint.disconnect()?;
    thread::sleep(Duration::from_secs(5));
    println!("Connecting again");
    endpoint.connect(address)?;
    eprintln!("Press Enter to stop.");

    // Enter or the end of input stops the program.
    let _ = std::io::stdin().read_line(&mut String::new());
    endpoint.close_and_await_termination()
}

/// The ADDRESS argument; without it, `dxfeed.address` of the `dxfeed.system.properties` file or of the
/// `DXFEED_dxfeed.address` environment variable, and the demo feed as the last resort.
fn address_or_default(address: Option<String>) -> dxfeed_graal::Result<String> {
    match address {
        Some(address) => Ok(address),
        None => Ok(system::get_property("dxfeed.address")?.unwrap_or_else(|| "demo.dxfeed.com:7300".into())),
    }
}
