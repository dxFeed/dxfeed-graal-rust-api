//! Prints the order book of a symbol, the best buy and sell orders side by side, until Enter is pressed, like
//! `MarketDepthModelSample` of the other dxFeed APIs. It builds a `MarketDepthModel` of orders on the default feed,
//! whose address is the `dxfeed.address` system property.
//!
//! Usage: `cargo run --example market_depth_model -- [SYMBOL] [SOURCES] [ADDRESS]`, by default `AAPL` and `NTV`;
//! SOURCES is a comma-separated list such as `ntv` or `AGGREGATE_ASK,AGGREGATE_BID`. Without ADDRESS the address is
//! `dxfeed.address` of the `dxfeed.system.properties` file in the current directory or of the
//! `DXFEED_dxfeed.address` environment variable, else `demo.dxfeed.com:7300`.

use std::process::ExitCode;
use std::time::Duration;

use dxfeed_graal::event::{IndexedEventSource, Order, OrderSource};
use dxfeed_graal::model::MarketDepthModel;
use dxfeed_graal::{DXEndpoint, DXFeed, system};

/// How many orders of each side are printed.
const DEPTH: usize = 10;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let symbol = args.next().unwrap_or_else(|| "AAPL".into());
    let sources = args.next().unwrap_or_else(|| "NTV".into());
    match run(&symbol, &sources, args.next()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(symbol: &str, sources: &str, address: Option<String>) -> dxfeed_graal::Result<()> {
    let address = address_or_default(address)?;
    let sources = sources
        .split(',')
        .map(OrderSource::value_of)
        .collect::<dxfeed_graal::Result<Vec<_>>>()?;
    // The default feed connects to `dxfeed.address` when it is first used.
    system::set_property("dxfeed.address", &address)?;

    let model = MarketDepthModel::<Order>::new_builder()
        .with_feed(DXFeed::get_instance()?)
        .with_symbol(symbol)
        .with_sources(sources)
        .with_depth_limit(DEPTH)
        // Updates are printed at most once a second; snapshots at once.
        .with_aggregation_period(Duration::from_secs(1))
        .with_listener(render)
        .build()?;
    println!("Subscribed to the order book of {symbol} from {address}; press Enter to stop.");

    // Enter or the end of input stops the program.
    let _ = std::io::stdin().read_line(&mut String::new());
    drop(model);
    DXEndpoint::get_instance()?.close_and_await_termination()
}

/// Prints the sides as a table: the buy orders on the left, the sell orders on the right.
fn render(buy: &[Order], sell: &[Order]) {
    let mut table = format!("{:=^78}\n{:^38}||{:^38}\n", "", "BID", "ASK");
    let header = format!("{:^2}|{:^13}|{:^4}|{:^8}|{:^7}", "EX", "Source", "MM", "Price", "Size");
    table += &format!("{header}||{header}\n{:-^78}\n", "");
    for i in 0..buy.len().max(sell.len()) {
        table += &format!("{:38}||{}\n", row(buy.get(i)), row(sell.get(i)));
    }
    print!("{table}");
}

/// A row of the table for an order, or blanks.
fn row(order: Option<&Order>) -> String {
    let Some(order) = order else {
        return String::new();
    };
    let source =
        IndexedEventSource::from_id(order.source()).map_or_else(|_| order.source().to_string(), |s| s.to_string());
    let exchange = order.exchange_code();
    format!(
        "{:^2}|{:^13}|{:^4}|{:^8}|{:^7}",
        if exchange == '\0' { ' ' } else { exchange },
        source,
        order.market_maker.as_deref().unwrap_or(""),
        order.price,
        order.size,
    )
}

/// The ADDRESS argument; without it, `dxfeed.address` of the `dxfeed.system.properties` file or of the
/// `DXFEED_dxfeed.address` environment variable, and the demo feed as the last resort.
fn address_or_default(address: Option<String>) -> dxfeed_graal::Result<String> {
    match address {
        Some(address) => Ok(address),
        None => Ok(system::get_property("dxfeed.address")?.unwrap_or_else(|| "demo.dxfeed.com:7300".into())),
    }
}
