//! Prints the state changes of an endpoint while it connects, disconnects and connects again, like `ReconnectSample`
//! of the other dxFeed APIs.
//!
//! Usage: `cargo run --example reconnect -- [ADDRESS]`, by default `demo.dxfeed.com:7300`.

use std::process::ExitCode;
use std::thread;
use std::time::Duration;

use dxfeed_graal::DXEndpoint;
use dxfeed_graal::event::Quote;

fn main() -> ExitCode {
    let address = std::env::args().nth(1).unwrap_or_else(|| "demo.dxfeed.com:7300".into());
    match run(&address) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(address: &str) -> dxfeed_graal::Result<()> {
    let endpoint = DXEndpoint::create()?;
    endpoint.add_state_listener(|old, new| println!("Connection state changed: {old}->{new}"))?;
    endpoint.connect(address)?;

    let subscription = endpoint.feed().create_subscription::<Quote>()?;
    subscription.add_event_listener(|quotes| {
        for quote in quotes {
            println!("{quote}");
        }
    })?;
    subscription.add_symbols(["ETH/USD:GDAX"])?;

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
