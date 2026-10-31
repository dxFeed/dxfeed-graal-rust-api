//! Demonstrates how to connect to an endpoint, subscribe to market data events, handle reconnections and
//! re-subscribing.
//!
//! Usage: `cargo run --example ReconnectSample`

use std::thread;
use std::time::Duration;

use dxfeed_graal::DXEndpoint;
use dxfeed_graal::event::Quote;

fn main() -> dxfeed_graal::Result<()> {
    let address = "demo.dxfeed.com:7300"; // The address of the DxFeed endpoint.
    let symbol = "ETH/USD:GDAX"; // The symbol for which we want to receive quotes.

    // Create a new endpoint and add a listener for state changes.
    let endpoint = DXEndpoint::create()?;

    endpoint.add_state_listener(|old_state, new_state| {
        println!("Connection state changed: {old_state}->{new_state}");
    })?;

    // Connect to the endpoint using the specified address.
    endpoint.connect(address)?;

    // Create a subscription for Quote events.
    let subscription = endpoint.get_feed().create_subscription::<Quote>()?;

    subscription.add_event_listener(|quotes| {
        // Event listener that prints each received event.
        for quote in quotes {
            println!("{quote}");
        }
    })?;

    // Add the specified symbol to the subscription.
    subscription.add_symbols([symbol])?;

    // Wait for five seconds to allow some quotes to be received.
    thread::sleep(Duration::from_secs(5));

    // Disconnect from the endpoint.
    endpoint.disconnect()?;

    // Wait for another five seconds to ensure quotes stop coming in.
    thread::sleep(Duration::from_secs(5));

    // Reconnect to the endpoint.
    // The subscription is automatically re-subscribed, and quotes start coming into the listener again.
    // Another address can also be passed on.
    endpoint.connect(address)?;

    // Keep the application running indefinitely.
    let _ = std::io::stdin().read_line(&mut String::new());
    Ok(())
}
