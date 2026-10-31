//! Using an address like `:7700`, starts a server on that port that publishes a `Profile` event for any symbol ending
//! with `:TEST` that its clients subscribe to, until Enter is pressed, like `PublishProfilesSample` of the dxFeed Java
//! API. The `request_profile` sample is such a client.
//!
//! Usage: `cargo run --example publish_profiles -- <ADDRESS>`, for example `:7700`.

use std::process::ExitCode;

use dxfeed_graal::event::Profile;
use dxfeed_graal::{DXEndpoint, Event, Role, Symbol};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [address] = args.as_slice() else {
        eprintln!("Usage: publish_profiles <address>");
        eprintln!("  address - the address of the server, for example :7700");
        return ExitCode::from(2);
    };
    match run(address) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(address: &str) -> dxfeed_graal::Result<()> {
    let endpoint = DXEndpoint::create_with_role(Role::Publisher)?;
    endpoint.connect(address)?;
    let publisher = endpoint.get_publisher();

    // The subscription of the publisher tells which symbols the clients ask the profiles of.
    let subscription = publisher.get_subscription::<Profile>()?;
    subscription.add_change_listener(move |symbols: &[Symbol]| {
        let profiles: Vec<Event> = symbols
            .iter()
            .filter_map(|symbol| match symbol {
                Symbol::String(symbol) if symbol.ends_with(":TEST") => Some(
                    Profile {
                        description: Some("Test symbol".into()),
                        ..Profile::new(symbol)
                    }
                    .into(),
                ),
                _ => None,
            })
            .collect();
        if let Err(error) = publisher.publish_events(&profiles) {
            eprintln!("error: {error}");
        }
    })?;
    eprintln!("Press Enter to stop.");

    // Enter or the end of input stops the program.
    let _ = std::io::stdin().read_line(&mut String::new());
    endpoint.close_and_await_termination()
}
