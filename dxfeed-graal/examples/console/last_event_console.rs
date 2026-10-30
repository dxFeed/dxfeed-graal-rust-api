//! Reads symbols from the console and prints the last quote, trade, summary and profile of each, like
//! `LastEventConsoleSample` of the other dxFeed APIs. An empty line, `exit`, `quit` or `q` stops it.
//!
//! The endpoint keeps a permanent subscription to these events for all symbols of the demo feed, so the
//! events are cached in memory and snapshots are taken from there; a symbol outside the cache is requested from the
//! uplink.
//!
//! Usage: `cargo run --example last_event_console -- [ADDRESS]`. Without ADDRESS the
//! address is `dxfeed.address` of the `dxfeed.system.properties` file in the current directory or of the
//! `DXFEED_dxfeed.address` environment variable, else `demo.dxfeed.com:7300`.

use std::fmt::Display;
use std::io::BufRead;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use dxfeed_graal::event::{LastingEvent, Profile, Quote, Summary, Trade};
use dxfeed_graal::{DXEndpoint, Promise, system};

fn main() -> ExitCode {
    match run(std::env::args().nth(1)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(address: Option<String>) -> dxfeed_graal::Result<()> {
    // The permanent subscription property `dxfeed.qd.subscribe.ticker` holds a comma-separated list of records (the
    // composite events have the names of their event types), a space and a comma-separated list of symbols. Here the
    // symbols are those of an instrument profile file: a URL containing ".ipf".
    let records = "Quote,Trade,Summary,Profile";
    let symbols = "https://dxfeed.s3.amazonaws.com/masterdata/ipf/demo/mux-demo.ipf.zip";
    // With `dxfeed.address` set, the endpoint connects when it is built; the property may also be given to the builder.
    system::set_property("dxfeed.address", &address_or_default(address)?)?;
    let endpoint = DXEndpoint::new_builder()
        .with_property("dxfeed.qd.subscribe.ticker", &format!("{records} {symbols}"))
        .build()?;
    let feed = endpoint.feed();

    println!("Type symbols to get their quote, trade, summary, and profile event snapshots");
    // The end of input stops the program too.
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        let symbol = line.trim();
        if matches!(symbol, "" | "exit" | "quit" | "q") {
            break;
        }

        // A promise returns an event even for a symbol that has no subscription yet.
        let quote = feed.get_last_event_promise::<Quote>(symbol)?;
        let trade = feed.get_last_event_promise::<Trade>(symbol)?;
        let summary = feed.get_last_event_promise::<Summary>(symbol)?;
        // Profiles are composite only: for a regional symbol such as "IBM&N" the request never completes.
        let profile = if has_exchange_code(symbol) {
            None
        } else {
            Some(feed.get_last_event_promise::<Profile>(symbol)?)
        };

        // The other APIs wait for all promises together for at most a second; here each promise waits for what is
        // left of that second. A promise still pending at the deadline prints as null.
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut completed = wait(&quote, deadline) & wait(&trade, deadline) & wait(&summary, deadline);
        if let Some(profile) = &profile {
            completed &= wait(profile, deadline);
        }
        if !completed {
            println!("Request timed out");
        }
        println!("{}", result(&quote));
        println!("{}", result(&trade));
        println!("{}", result(&summary));
        if let Some(profile) = &profile {
            println!("{}", result(profile));
        }
    }
    endpoint.close_and_await_termination()
}

/// Waits for the promise until the deadline; whether it completed.
fn wait<E: LastingEvent>(promise: &Promise<E>, deadline: Instant) -> bool {
    promise.wait(deadline.saturating_duration_since(Instant::now())).is_ok()
}

/// The event of the promise, or `null` as Java prints a missing one.
fn result<E: LastingEvent + Display>(promise: &Promise<E>) -> String {
    match promise.result() {
        Ok(Some(event)) => event.to_string(),
        _ => "null".into(),
    }
}

/// Whether the symbol names a regional exchange, such as "IBM&N" or "IBM&N{=d}" (`MarketEventSymbols.hasExchangeCode`).
fn has_exchange_code(symbol: &str) -> bool {
    let base = match symbol.strip_suffix('}').and_then(|_| symbol.rfind('{')) {
        Some(attributes) => &symbol[..attributes],
        None => symbol,
    };
    let bytes = base.as_bytes();
    bytes.len() >= 2 && bytes[bytes.len() - 2] == b'&' && bytes[bytes.len() - 1].is_ascii_alphabetic()
}

/// The ADDRESS argument; without it, `dxfeed.address` of the `dxfeed.system.properties` file or of the
/// `DXFEED_dxfeed.address` environment variable, and the demo feed as the last resort.
fn address_or_default(address: Option<String>) -> dxfeed_graal::Result<String> {
    match address {
        Some(address) => Ok(address),
        None => Ok(system::get_property("dxfeed.address")?.unwrap_or_else(|| "demo.dxfeed.com:7300".into())),
    }
}
