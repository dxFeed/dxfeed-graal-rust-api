//! Creates multiple event listener and subscribe to Quote and Trade events.
//! Use default DXFeed instance for that data feed address is defined by "dxfeed.system.properties" file.
//! The properties file is read from the current directory, so run the sample from its directory:
//! `cargo run --example QuoteAndTradeSample -- <symbol>`.

use std::thread;
use std::time::{Duration, Instant};

use dxfeed_graal::event::{EventRef, EventType, Quote, Trade};
use dxfeed_graal::{DXFeed, DXFeedSubscription, Event};

// Unlike C++, the subscription is returned: dropping a subscription closes it.
fn test_quote_listener(symbol: &str) -> dxfeed_graal::Result<DXFeedSubscription<Quote>> {
    // Creates a subscription attached to a default DXFeed with a Quote event type.
    // The endpoint address to use is stored in the "dxfeed.system.properties" file.
    let subscription = DXFeed::get_instance()?.create_subscription::<Quote>()?;

    // Listener must be attached before symbols are added.
    // Filtered by quotes
    subscription.add_event_listener(|quotes| {
        for q in quotes {
            println!("Mid = {:.6}", (q.bid_price() + q.ask_price()) / 2.0);
        }
    })?;

    // C++ also shows another way to subscribe, with MarketEvent. Rust has no MarketEvent-typed subscription, so that
    // alternative is left out.

    // Adds specified symbol.
    subscription.add_symbols([symbol])?;

    Ok(subscription)
}

fn test_quote_promises(symbol: &str) -> dxfeed_graal::Result<()> {
    // Creates a subscription attached to a default DXFeed with a Quote event type.
    // The endpoint address to use is stored in the "dxfeed.system.properties" file.

    let feed = DXFeed::get_instance()?;
    let symbols = [symbol];
    let promises = symbols
        .into_iter()
        .map(|symbol| feed.get_last_event_promise::<Quote>(symbol))
        .collect::<dxfeed_graal::Result<Vec<_>>>()?;

    // There is no Promises::allOf in Rust: wait for every promise, all within the same 5 seconds, without an exception
    let deadline = Instant::now() + Duration::from_secs(5);

    for promise in &promises {
        let _ = promise.wait(deadline.saturating_duration_since(Instant::now()));
    }

    // now iterate the promises to retrieve results
    // a promise that is not done by the deadline has no result
    for (symbol, promise) in symbols.into_iter().zip(&promises) {
        match promise.get_result() {
            Ok(Some(q)) => println!("Mid = {:.6}", (q.bid_price + q.ask_price) / 2.0),
            _ => println!("No quote for {symbol}"),
        }
    }

    Ok(())
}

// Unlike C++, the subscription is returned: dropping a subscription closes it.
fn test_quote_and_trade_listener(symbol: &str) -> dxfeed_graal::Result<DXFeedSubscription<Event>> {
    // Creates a subscription attached to a default DXFeed with Quote and Trade event types.
    // The endpoint address to use is stored in the "dxfeed.system.properties" file.
    let subscription = DXFeed::get_instance()?.create_subscription_with_types(&[EventType::Quote, EventType::Trade])?;

    // Listener must be attached before symbols are added.
    subscription.add_event_listener(|events| {
        for e in events {
            // An example of matching the type of an event
            match e {
                EventRef::Quote(q) => println!("{q}"),
                EventRef::Trade(t) => println!("{t}"),
                _ => {}
            }
        }
    })?;

    // Adds specified symbol.
    subscription.add_symbols([symbol])?;

    Ok(subscription)
}

fn test_trade_snapshots(symbol: &str) -> dxfeed_graal::Result<()> {
    let feed = DXFeed::get_instance()?;
    let sub = feed.create_subscription::<Trade>()?;

    sub.add_symbols([symbol])?;

    loop {
        println!("LAST: {}", feed.get_last_event(&Trade::new(symbol))?);
        thread::sleep(Duration::from_secs(1));
    }
}

fn main() -> dxfeed_graal::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        print!(
            r"
Usage:
QuoteAndTradeSample <symbol>

Where:
    symbol - Is security symbol (e.g. IBM, AAPL, SPX etc.)
"
        );

        return Ok(());
    }

    // Specified instrument name, for example AAPL, IBM, MSFT, etc.
    let symbol = &args[1];

    let _quote_subscription = test_quote_listener(symbol)?;
    test_quote_promises(symbol)?;
    let _quote_and_trade_subscription = test_quote_and_trade_listener(symbol)?;
    test_trade_snapshots(symbol)
}
