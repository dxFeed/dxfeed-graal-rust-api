//! Usage: `cargo run --bin WriteTapeFileSample`

use dxfeed_graal::event::Quote;
use dxfeed_graal::{DXEndpoint, Event, Role};

/// Write events to a tape file.
fn main() -> dxfeed_graal::Result<()> {
    // Create an appropriate endpoint.
    let endpoint = DXEndpoint::new_builder()
        // Is required for tape connector to be able to receive everything.
        .with_property("dxfeed.wildcard.enable", "true")
        .with_role(Role::Publisher)
        .build()?;

    // Connect to the address, remove [format=text] or change on [format=binary] for binary format
    endpoint.connect("tape:WriteTapeFile.out.txt[format=text]")?;

    // Get a publisher.
    let pubr = endpoint.get_publisher();

    // Creates new Quote market events.
    let quote1 = Quote {
        bid_price: 10.1,
        ask_price: 10.2,
        ..Quote::new("TEST1")
    };
    let quote2 = Quote {
        bid_price: 17.1,
        ask_price: 17.2,
        ..Quote::new("TEST2")
    };

    // Publish events.
    pubr.publish_events(&[Event::Quote(quote1), Event::Quote(quote2)])?;

    // Wait until all data is written, close, and wait until it closes.
    endpoint.await_processed()?;
    endpoint.close_and_await_termination()?;

    Ok(())
}
