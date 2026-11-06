//! Using address like ":7700" it starts a server on a specified port where it provides a `Profile` event for any
//! symbol ending with ":TEST" suffix.
//!
//! Usage: `cargo run --bin PublishProfilesSample -- <address>`

use dxfeed_graal::event::Profile;
use dxfeed_graal::{DXEndpoint, Event, Role, Symbol};

fn main() -> dxfeed_graal::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        return Ok(());
    }

    // Parse args.
    let address = &args[1];

    // Create a publisher endpoint and connect it to the specified address
    let endpoint = DXEndpoint::create_with_role(Role::Publisher)?;
    endpoint.connect(address)?;
    let publisher = endpoint.get_publisher();

    // Observe Profile subscriptions and publish profiles for "xxx:TEST" symbols
    publisher
        .get_subscription::<Profile>()?
        .add_change_listener(move |symbols: &[Symbol]| /* symbols_added */ {
            let mut events: Vec<Event> = Vec::with_capacity(symbols.len());

            for symbol in symbols {
                if let Symbol::String(s) = symbol
                    && s.len() > 5
                    && s.ends_with(":TEST")
                {
                    let mut profile = Profile::new(s);

                    profile.description = Some("Test symbol".into());
                    events.push(profile.into());
                }
            }

            events.shrink_to_fit();

            if let Err(e) = publisher.publish_events(&events) {
                eprintln!("{e}");
            }
        })?;

    let _ = std::io::stdin().read_line(&mut String::new());
    Ok(())
}
