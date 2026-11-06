//! A simple sample that shows how to subscribe to quotes for one instrument and print all received quotes to the
//! console. Use the default DXFeed instance for that data feed address is defined by "dxfeed.system.properties" file.
//! The properties file is read from the current directory, so run the sample from its directory:
//! `cargo run --bin PrintQuoteEventsSample -- [SYMBOL]`.

use dxfeed_graal::event::Quote;
use dxfeed_graal::{DXFeed, ErrorManager};

fn main() -> dxfeed_graal::Result<()> {
    // Specified instrument name, for example AAPL, IBM, MSFT, etc.
    let symbol = std::env::args().nth(1).unwrap_or_else(|| "AAPL".into());

    // Errors that have no caller go to ErrorManager: for example, a panic of the listener below, which runs on a
    // thread of the SDK. The default handler writes one line per error to stderr; this one adds a prefix.
    // The handler is called on the thread of the error and must be thread-safe.
    ErrorManager::set_handler(|error| eprintln!("PrintQuoteEventsSample: {error}"));

    // Creates a subscription attached to a default DXFeed with a Quote event type.
    // The endpoint address to use is stored in the "dxfeed.system.properties" file.
    let subscription = DXFeed::get_instance()?.create_subscription::<Quote>()?;
    // Listener must be attached before symbols are added.
    subscription.add_event_listener(|events| {
        // Prints all received events.
        for e in events {
            println!("{e}");
        }
    })?;
    // Adds specified symbol.
    subscription.add_symbols([symbol])?;
    let _ = std::io::stdin().read_line(&mut String::new());
    Ok(())
}
