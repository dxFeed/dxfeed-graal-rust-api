//! This sample demonstrates a way to subscribe to the big world of symbols with dxFeed API, so that the events are
//! updated and cached in memory of this process and then take snapshots of those events from memory whenever
//! they are needed. This example repeatedly reads the symbol name from the console and prints a snapshot of its last
//! quote, trade, summary, and profile events.
//! The properties file is read from the current directory, so run the sample from its directory:
//! `cargo run --example LastEventConsoleSample`.

use std::fmt::Display;
use std::time::{Duration, Instant};

use dxfeed_graal::event::{LastingEvent, Profile, Quote, Summary, Trade};
use dxfeed_graal::{DXEndpoint, Promise};

fn main() -> dxfeed_graal::Result<()> {
    // Permanent subscription to the world is performed with a special property named "dxfeed.qd.subscribe.ticker".
    // Its value consists of a comma-separated list of records, followed by a space, followed by a comma-separated
    // list of symbols. Record names for composite (NBBO) events are the same as the corresponding event classes
    // in API. The string below defines subscription for quote, trade, summary, and profile composite events:
    let records = "Quote,Trade,Summary,Profile";

    // Records for regional-exchange events are derived by appending "&" (ampersand) and a single-digit
    // exchange code. Regexp-like syntax can be used instead of listing them all. For example, the commented
    // line below and to the mix a subscription on regional quotes from all potential exchange codes A-Z
    // let records = "Quote,Trade,Summary,Profile,Quote&[A-Z]";

    // There is an important trade-off between resource consumption and speed of access to the last events.
    // The whole world of stocks and options from all the exchanges is very large and will consume gigabytes
    // of memory to cache. Moreover, this cache has to be constantly kept uptodate, which consumes a lot of
    // networks and CPU.
    //
    // A particular application's use cases have to be studied to figure out what is an option for this particular
    // application. If some events are known to be rarely needed and a small delay while access to them can be
    // tolerated, then it is not worth configuring a permanent subscription for them. The code in this
    // sample works using `DXFeed::get_last_event_promise()` method that will request an event from the upstream data
    // provider if it is not present in the local in-memory cache.

    // There are multiple ways to specify a list of symbols. It is typically taken from an IPF file, and its
    // specification consists of a URL to the file which has to contain ".ipf" to be recognized.
    // The string below defines subscription for all symbols that are available on the demo feed.
    let symbols = "https://dxfeed.s3.amazonaws.com/masterdata/ipf/demo/mux-demo.ipf.zip";

    // Permanent subscription property "dxfeed.qd.subscribe.ticker" can be directly placed into the
    // "dxfeed.system.properties" file, and no custom DXEndpoint instance will be needed. Here it is explicitly
    // specified using a DXEndpointBuilder class. Note that we don't use `DXEndpoint::connect()` method.
    // It is assumed by this sample that the ` dxfeed.address ` property is specified in "dxfeed.system.properties" and
    // connection is automatically established to that address.
    // Alternatively, "dxfeed.address" can also be specified in the builder.
    let endpoint = DXEndpoint::new_builder()
        .with_property("dxfeed.qd.subscribe.ticker", &format!("{records} {symbols}"))
        .build()?;

    // The actual client code does not need a reference to DXEndpoint, which only contains lifecycle
    // methods like "connect" and "close". The client code needs a reference to DXFeed.
    let feed = endpoint.get_feed();

    // Print a short help.
    println!("Type symbols to get their quote, trade, summary, and profile event snapshots");

    loop {
        // User of this sample application can type symbols in the console. Symbol like "IBM" corresponds
        // to the stock. Symbol like "IBM&N" corresponds to the information from a specific exchange.
        // See the dxFeed Symbol guide at http://www.dxfeed.com/downloads/documentation/dxFeed_Symbol_Guide.pdf
        let mut symbol = String::new();

        let _ = std::io::stdin().read_line(&mut symbol);
        let symbol = symbol.trim_end_matches(['\r', '\n']);

        if symbol.is_empty() || symbol == "exit" || symbol == "quit" || symbol == "q" {
            return Ok(());
        }

        // The first step is to extract promises for all events that we are interested in. This way we
        // can get an event even if we have not previously subscribed for it.
        let quote_promise = feed.get_last_event_promise::<Quote>(symbol)?;
        let trade_promise = feed.get_last_event_promise::<Trade>(symbol)?;
        let summary_promise = feed.get_last_event_promise::<Summary>(symbol)?;

        // Profile events are composite-only. They are not available for regional symbols like
        // "IBM&N" and the attempt to retrieve never completes (will time out), so we don't even try.
        let profile_promise = if !has_exchange_code(symbol) {
            Some(feed.get_last_event_promise::<Profile>(symbol)?)
        } else {
            None
        };

        // If the events are available in the in-memory cache, then the promises will be completed immediately.
        // Otherwise, a request to the upstream data provider is sent. Below we wait for at most 1 second for all the
        // promises to complete (`Promises.allOf(...).awaitWithoutException(1, TimeUnit.SECONDS)` of the Java API
        // has no counterpart here, so every promise waits for what is left of that second). The last event promise
        // never completes exceptionally, and we don't have to especially process a case of timeout, so the timeout
        // is not an error here: execution continues normally. This sample prints a special message in the case of
        // timeout.
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut completed = await_without_exception(&quote_promise, deadline)
            & await_without_exception(&trade_promise, deadline)
            & await_without_exception(&summary_promise, deadline);

        if let Some(profile_promise) = &profile_promise {
            completed &= await_without_exception(profile_promise, deadline);
        }

        if !completed {
            println!("Request timed out");
        }

        // The combination above is used only to ensure a common wait of 1 second. Promises to individual events
        // are completed independently, and the corresponding events can be accessed even if some events were not
        // available for any reason and the wait above had timed out. This sample just prints all results.
        // "<null>" is printed when the event is not available.
        println!("{}", result_to_string(&quote_promise));
        println!("{}", result_to_string(&trade_promise));
        println!("{}", result_to_string(&summary_promise));

        if let Some(profile_promise) = &profile_promise {
            println!("{}", result_to_string(profile_promise));
        }
    }
}

/// Waits for the promise until the deadline; whether it completed in time.
fn await_without_exception<E: LastingEvent>(promise: &Promise<E>, deadline: Instant) -> bool {
    promise.wait(deadline.saturating_duration_since(Instant::now())).is_ok()
}

/// The text of the event of the promise, or `<null>` when the event is not available.
fn result_to_string<E: LastingEvent + Display>(promise: &Promise<E>) -> String {
    match promise.get_result() {
        Ok(Some(event)) => event.to_string(),
        _ => "<null>".into(),
    }
}

/// Whether the symbol names a regional exchange, such as "IBM&N" or "IBM&N{=d}" (`MarketEventSymbols::hasExchangeCode`).
fn has_exchange_code(symbol: &str) -> bool {
    let base = match symbol.strip_suffix('}').and_then(|_| symbol.rfind('{')) {
        Some(attributes) => &symbol[..attributes],
        None => symbol,
    };
    let bytes = base.as_bytes();

    bytes.len() >= 2 && bytes[bytes.len() - 2] == b'&' && bytes[bytes.len() - 1].is_ascii_alphabetic()
}
