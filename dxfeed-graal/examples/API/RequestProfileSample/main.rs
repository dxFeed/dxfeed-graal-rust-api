//! Using address like "localhost:7700" and a symbol list "A:TEST" it connects to the running PublishProfilesSample,
//! prints symbol profile event, and exits.
//!
//! Usage: `cargo run --example RequestProfileSample -- <address> <symbol>`

use std::time::Duration;

use dxfeed_graal::event::Profile;
use dxfeed_graal::{DXEndpoint, Role};

fn main() -> dxfeed_graal::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 3 {
        return Ok(());
    }

    // Parse args.
    let address = &args[1];
    let symbol = &args[2];
    let endpoint = DXEndpoint::create_with_role(Role::Feed)?;
    endpoint.connect(address)?;
    let profile = endpoint
        .get_feed()
        .get_last_event_promise::<Profile>(symbol)?
        .wait(Duration::from_secs(5))?;

    println!("{profile}");
    endpoint.close()?;

    Ok(())
}
