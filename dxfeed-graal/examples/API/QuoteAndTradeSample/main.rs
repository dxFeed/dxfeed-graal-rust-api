//! Shows several ways to receive quotes and trades of a symbol from the default feed, like `QuoteAndTradeSample` of
//! the other dxFeed APIs: a typed listener, a last-event promise, an untyped listener for two event types and
//! snapshots of the last trade. Runs until the process is killed.
//!
//! Usage: `cargo run --example QuoteAndTradeSample -- <SYMBOL> [ADDRESS]`. Without ADDRESS the
//! address is `dxfeed.address` of the `dxfeed.system.properties` file in the current directory or of the
//! `DXFEED_dxfeed.address` environment variable, else `demo.dxfeed.com:7300`.

use std::process::ExitCode;
use std::thread;
use std::time::Duration;

use dxfeed_graal::event::{Event, EventRef, EventType, Quote, Trade};
use dxfeed_graal::{DXFeed, DXFeedSubscription, system};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(symbol) = args.next() else {
        eprintln!("Usage: quote_and_trade <symbol> [address]");
        eprintln!("    symbol  - a security symbol, such as IBM, AAPL or SPX");
        eprintln!("    address - the address to connect to; dxfeed.address of dxfeed.system.properties by default");
        return ExitCode::from(2);
    };
    match run(&symbol, args.next()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(symbol: &str, address: Option<String>) -> dxfeed_graal::Result<()> {
    // The default feed connects to `dxfeed.address` when it is first used.
    system::set_property("dxfeed.address", &address_or_default(address)?)?;
    let feed = DXFeed::get_instance()?;
    // Dropping a subscription closes it, so both are kept until the end.
    let _quotes = test_quote_listener(feed, symbol)?;
    test_quote_promise(feed, symbol)?;
    let _quotes_and_trades = test_quote_and_trade_listener(feed, symbol)?;
    test_trade_snapshots(feed, symbol)
}

fn test_quote_listener(feed: &DXFeed, symbol: &str) -> dxfeed_graal::Result<DXFeedSubscription<Quote>> {
    let subscription = feed.create_subscription::<Quote>()?;
    // Listeners go before symbols, so the first events are not missed.
    subscription.add_event_listener(|quotes| {
        for quote in quotes {
            println!("Mid = {}", (quote.bid_price() + quote.ask_price()) / 2.0);
        }
    })?;
    subscription.add_symbols([symbol])?;
    Ok(subscription)
}

fn test_quote_promise(feed: &DXFeed, symbol: &str) -> dxfeed_graal::Result<()> {
    let quote = feed
        .get_last_event_promise::<Quote>(symbol)?
        .wait(Duration::from_secs(5))?;
    println!("Mid = {}", (quote.bid_price + quote.ask_price) / 2.0);
    Ok(())
}

fn test_quote_and_trade_listener(feed: &DXFeed, symbol: &str) -> dxfeed_graal::Result<DXFeedSubscription<Event>> {
    let subscription = feed.create_subscription_with_types(&[EventType::Quote, EventType::Trade])?;
    subscription.add_event_listener(|events| {
        for event in events {
            match event {
                EventRef::Quote(quote) => println!("{quote}"),
                EventRef::Trade(trade) => println!("{trade}"),
                _ => {}
            }
        }
    })?;
    subscription.add_symbols([symbol])?;
    Ok(subscription)
}

fn test_trade_snapshots(feed: &DXFeed, symbol: &str) -> dxfeed_graal::Result<()> {
    // The cache of the feed keeps the last trade only for subscribed symbols.
    let subscription = feed.create_subscription::<Trade>()?;
    subscription.add_symbols([symbol])?;
    let trade = Trade::new(symbol);
    loop {
        println!("LAST: {}", feed.get_last_event(&trade)?);
        thread::sleep(Duration::from_secs(1));
    }
}

/// The ADDRESS argument; without it, `dxfeed.address` of the `dxfeed.system.properties` file or of the
/// `DXFEED_dxfeed.address` environment variable, and the demo feed as the last resort.
fn address_or_default(address: Option<String>) -> dxfeed_graal::Result<String> {
    match address {
        Some(address) => Ok(address),
        None => Ok(system::get_property("dxfeed.address")?.unwrap_or_else(|| "demo.dxfeed.com:7300".into())),
    }
}
