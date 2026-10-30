//! Prints the quotes of a symbol until Enter is pressed, like `PrintQuoteEventsSample` of the other dxFeed APIs.
//!
//! Usage: `cargo run --example print_quote_events -- [SYMBOL] [ADDRESS]`, by default `AAPL` and `demo.dxfeed.com:7300`.

use std::process::ExitCode;

use dxfeed_graal::DXEndpoint;
use dxfeed_graal::event::Quote;

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
    let endpoint = DXEndpoint::create()?;
    let subscription = endpoint.feed().create_subscription::<Quote>()?;
    // Listeners go before symbols, so the first quotes are not missed.
    subscription.add_event_listener(|quotes| {
        for quote in quotes {
            println!("{quote}");
        }
    })?;
    subscription.add_symbols([symbol])?;
    endpoint.connect(address)?;
    eprintln!("Printing quotes of {symbol} from {address}; press Enter to stop.");

    // Enter or the end of input stops the program.
    let _ = std::io::stdin().read_line(&mut String::new());
    endpoint.close_and_await_termination()
}
