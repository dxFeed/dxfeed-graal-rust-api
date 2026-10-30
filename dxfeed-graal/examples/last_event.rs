//! Prints the last quote of a symbol, requested with a promise and waited for with a timeout.
//!
//! Usage: `cargo run --example last_event -- [SYMBOL] [ADDRESS]`, by default `AAPL` and `demo.dxfeed.com:7300`.

use std::process::ExitCode;
use std::time::Duration;

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
    endpoint.connect(address)?;
    let promise = endpoint.feed().get_last_event_promise::<Quote>(symbol)?;
    // A timeout cancels the request and returns `Error::Timeout`.
    let quote = promise.wait(Duration::from_secs(10))?;
    println!("{quote}");
    endpoint.close_and_await_termination()
}
