//! Demonstrates how to connect to an endpoint requires authentication token, subscribe to market data events, and
//! handle periodic token updates.
//!
//! Usage: `cargo run --bin AuthSample`

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::thread;
use std::time::Duration;

use dxfeed_graal::event::Quote;
use dxfeed_graal::{DXEndpoint, DXFeed};

fn main() -> dxfeed_graal::Result<()> {
    let address = "demo.dxfeed.com:7300";

    let generate_token = || {
        let length = random_in(4, 9);

        (0..length)
            .map(|_| char::from(random_in(usize::from(b'a'), usize::from(b'z')) as u8))
            .collect::<String>()
    };

    // Add a listener for state changes to the default application-wide singleton instance of DXEndpoint.
    DXEndpoint::get_instance()?.add_state_listener(|old_state, new_state| {
        println!("Connection state changed: {old_state}->{new_state}");
    })?;

    // Set up a timer to periodically update the token and reconnect every 10 seconds.
    // The first connection will be made immediately.
    // After reconnection, all existing subscriptions will be re-subscribed automatically.
    let _update_token_timer = thread::spawn(move || {
        loop {
            let connected = DXEndpoint::get_instance()
                .and_then(|endpoint| endpoint.connect(&format!("{address}[login=entitle:{}]", generate_token())));

            if let Err(e) = connected {
                eprintln!("{e}");
            }

            thread::sleep(Duration::from_secs(10));
        }
    });

    // Create a subscription for Quote events.
    let subscription = DXFeed::get_instance()?.create_subscription::<Quote>()?;

    subscription.add_event_listener(|quotes| {
        // Event listener that prints each received event.
        for quote in quotes {
            println!("{quote}");
        }
    })?;

    // Add the specified symbol to the subscription.
    subscription.add_symbols(["ETH/USD:GDAX"])?;

    let _ = std::io::stdin().read_line(&mut String::new());
    Ok(())
}

/// A random number from `low` to `high` inclusive, like `std::uniform_int_distribution`: the Rust standard library
/// has no random number generator, but each `RandomState` has new random keys, which is enough for a sample.
fn random_in(low: usize, high: usize) -> usize {
    low + RandomState::new().build_hasher().finish() as usize % (high - low + 1)
}
