//! Builds the order book of a symbol and an order source from the transactions of an `IndexedEventTxModel` and prints
//! its best orders every second until Enter is pressed, like `SimpleOrderBookSample` of the other dxFeed APIs. The
//! model works on the default feed, whose address is the `dxfeed.address` system property.
//!
//! Usage: `cargo run --example SimpleOrderBookSample -- [SYMBOL] [SOURCE] [ADDRESS]`, by default `AAPL` and `ntv`. Without
//! ADDRESS the address is `dxfeed.address` of the `dxfeed.system.properties` file in the current directory or of the
//! `DXFEED_dxfeed.address` environment variable, else `demo.dxfeed.com:7300`.

use std::collections::HashMap;
use std::process::ExitCode;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use dxfeed_graal::event::{EventFlags, IndexedEventSource, Order, OrderEvent, OrderSource, Side};
use dxfeed_graal::model::IndexedEventTxModel;
use dxfeed_graal::{DXEndpoint, DXFeed, system};

/// How many orders of each side are printed.
const DEPTH: usize = 5;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let symbol = args.next().unwrap_or_else(|| "AAPL".into());
    let source = args.next().unwrap_or_else(|| "ntv".into());
    match run(&symbol, &source, args.next()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(symbol: &str, source: &str, address: Option<String>) -> dxfeed_graal::Result<()> {
    let address = address_or_default(address)?;
    let source = OrderSource::value_of(source)?;
    // The default feed connects to `dxfeed.address` when it is first used.
    system::set_property("dxfeed.address", &address)?;

    // The orders of the book by their indices; each transaction or snapshot comes whole.
    let book = Arc::new(Mutex::new(HashMap::<i64, Order>::new()));
    let orders = Arc::clone(&book);
    let model = IndexedEventTxModel::<Order>::new_builder()
        .with_feed(DXFeed::get_instance()?)
        .with_symbol(symbol)
        .with_source(source)
        .with_listener(move |events, is_snapshot| {
            let mut orders = orders.lock().unwrap();
            if is_snapshot {
                orders.clear();
            }
            for order in events {
                let order = order.to_owned();
                // A removed order or one without a size removes the order of its index.
                if order.has_size() && !order.event_flags.contains(EventFlags::REMOVE_EVENT) {
                    orders.insert(order.index, order);
                } else {
                    orders.remove(&order.index);
                }
            }
        })
        .build()?;
    println!("Subscribed to the {source} orders of {symbol} from {address}; press Enter to stop.");

    // The book is printed every second until Enter or the end of input.
    let (stop, stopped) = mpsc::channel::<()>();
    let printer = thread::spawn(move || {
        while let Err(RecvTimeoutError::Timeout) = stopped.recv_timeout(Duration::from_secs(1)) {
            print_book(&book.lock().unwrap());
        }
    });
    let _ = std::io::stdin().read_line(&mut String::new());
    drop(stop);
    let _ = printer.join();
    drop(model);
    DXEndpoint::get_instance()?.close_and_await_termination()
}

/// Prints the best buy and sell orders side by side: buy orders by price descending, sell orders by price ascending,
/// orders of one price by index.
fn print_book(orders: &HashMap<i64, Order>) {
    let mut buy: Vec<_> = orders
        .values()
        .filter(|order| order.order_side() == Side::Buy)
        .collect();
    let mut sell: Vec<_> = orders
        .values()
        .filter(|order| order.order_side() == Side::Sell)
        .collect();
    buy.sort_by(|a, b| b.price.total_cmp(&a.price).then_with(|| a.index.cmp(&b.index)));
    sell.sort_by(|a, b| a.price.total_cmp(&b.price).then_with(|| a.index.cmp(&b.index)));
    for i in 0..DEPTH {
        println!("BUY {}\tSELL {}", describe(buy.get(i)), describe(sell.get(i)));
    }
    println!();
}

/// The source, size and price of an order, or `[None]`.
fn describe(order: Option<&&Order>) -> String {
    match order {
        Some(order) => {
            let source = IndexedEventSource::from_id(order.source())
                .map_or_else(|_| order.source().to_string(), |source| source.to_string());
            format!(
                "[Source: {source}, Size: {:8.4}, Price: {:8.2}]",
                order.size, order.price
            )
        }
        None => "[None]".into(),
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
