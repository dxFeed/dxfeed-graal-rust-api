//! Connects the default endpoint with a new random login token every 10 seconds and prints its state changes and
//! quotes, like `AuthSample` of the other dxFeed APIs. Each new token makes the endpoint connect again.
//!
//! Usage: `cargo run --example AuthSample -- [SYMBOL] [ADDRESS]`, by default `ETH/USD:GDAX`. Without ADDRESS the
//! address is `dxfeed.address` of the `dxfeed.system.properties` file in the current directory or of the
//! `DXFEED_dxfeed.address` environment variable, else `demo.dxfeed.com:7300`.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::process::ExitCode;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use dxfeed_graal::event::Quote;
use dxfeed_graal::{DXEndpoint, DXFeed, system};

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
    let endpoint = DXEndpoint::get_instance()?;
    endpoint.add_state_listener(|old, new| println!("Connection state changed: {old}->{new}"))?;

    let subscription = DXFeed::get_instance()?.create_subscription::<Quote>()?;
    subscription.add_event_listener(|quotes| {
        for quote in quotes {
            println!("{quote}");
        }
    })?;
    subscription.add_symbols([symbol])?;

    // Enter or the end of input stops the program.
    let (stop, stopped) = mpsc::channel();
    thread::spawn(move || {
        let _ = std::io::stdin().read_line(&mut String::new());
        let _ = stop.send(());
    });
    eprintln!("Press Enter to stop.");
    loop {
        let token = random_token();
        println!("Connecting with token {token}");
        endpoint.connect(&format!("{address}[login=entitle:{token}]"))?;
        match stopped.recv_timeout(Duration::from_secs(10)) {
            Err(RecvTimeoutError::Timeout) => {}
            Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    endpoint.close_and_await_termination()
}

/// From 4 to 9 random lowercase letters.
fn random_token() -> String {
    // Each `RandomState` has new random keys, which is enough for a sample.
    let mut random = RandomState::new().build_hasher().finish();
    let length = 4 + random % 6;
    (0..length)
        .map(|_| {
            random /= 6;
            let letter = b'a' + (random % 26) as u8;
            random /= 26;
            letter as char
        })
        .collect()
}

/// The ADDRESS argument; without it, `dxfeed.address` of the `dxfeed.system.properties` file or of the
/// `DXFEED_dxfeed.address` environment variable, and the demo feed as the last resort.
fn address_or_default(address: Option<String>) -> dxfeed_graal::Result<String> {
    match address {
        Some(address) => Ok(address),
        None => Ok(system::get_property("dxfeed.address")?.unwrap_or_else(|| "demo.dxfeed.com:7300".into())),
    }
}
