//! Prints the quotes of a symbol until Enter is pressed.
//!
//! Usage: `cargo run --example print_quotes -- [SYMBOL] [ADDRESS]`, by default `AAPL` and `demo.dxfeed.com:7300`.

use std::process::ExitCode;

use dxfeed_graal::{DXEndpoint, EventRef, EventType, Role, system};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let symbol = args.next().unwrap_or_else(|| "AAPL".into());
    let address = args.next().unwrap_or_else(|| "demo.dxfeed.com:7300".into());
    match run(&symbol, &address) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(symbol: &str, address: &str) -> dxfeed_graal::Result<()> {
    let endpoint = DXEndpoint::create_with_role(Role::Feed)?;
    let subscription = endpoint.feed().create_subscription(&[EventType::Quote])?;
    // Listeners go before symbols, so the first quotes are not missed.
    subscription.add_event_listener(|events| {
        for event in events {
            if let EventRef::Quote(q) = event {
                println!(
                    "{} bid {} x {} ({}) ask {} x {} ({})",
                    q.symbol(),
                    q.bid_price(),
                    q.bid_size(),
                    q.bid_exchange_code(),
                    q.ask_price(),
                    q.ask_size(),
                    q.ask_exchange_code(),
                );
            }
        }
    })?;
    subscription.add_symbols(&[symbol])?;
    endpoint.connect(address)?;
    println!("Printing quotes of {symbol} from {address}; press Enter to stop.");

    // Enter or the end of input stops the program.
    let _ = std::io::stdin().read_line(&mut String::new());
    system::shutdown()
}
