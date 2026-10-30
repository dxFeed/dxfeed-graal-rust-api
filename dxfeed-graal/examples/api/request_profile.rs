//! Connects to an address, prints the profile of a symbol and exits, like `RequestProfileSample` of the other dxFeed
//! APIs. With a running `publish_profiles` sample, use an address like `localhost:7700` and a symbol like `A:TEST`.
//!
//! Usage: `cargo run --example request_profile -- <ADDRESS> <SYMBOL>`, for example `demo.dxfeed.com:7300 AAPL`.

use std::process::ExitCode;
use std::time::Duration;

use dxfeed_graal::DXEndpoint;
use dxfeed_graal::event::Profile;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [address, symbol] = args.as_slice() else {
        eprintln!("Usage: request_profile <address> <symbol>");
        return ExitCode::from(2);
    };
    match run(address, symbol) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(address: &str, symbol: &str) -> dxfeed_graal::Result<()> {
    let endpoint = DXEndpoint::create()?;
    endpoint.connect(address)?;
    let profile = endpoint
        .feed()
        .get_last_event_promise::<Profile>(symbol)?
        .wait(Duration::from_secs(5))?;
    println!("{profile}");
    endpoint.close()
}
