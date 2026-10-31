//! Creates an endpoint and a quote subscription without connecting, then closes them.
//!
//! Its build script deploys the SDK library next to the executable, so the executable runs from any directory that
//! holds both files. `cargo run -p xtask -- check-deploy` checks exactly that.

use std::process::ExitCode;

use dxfeed_graal::DXEndpoint;
use dxfeed_graal::event::Quote;

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
    let endpoint = DXEndpoint::create()?;
    let subscription = endpoint.get_feed().create_subscription::<Quote>()?;
    subscription.add_symbols(["AAPL"])?;
    println!("Created a {:?} endpoint with a quote subscription", endpoint.get_role());
    endpoint.close_and_await_termination()
}
