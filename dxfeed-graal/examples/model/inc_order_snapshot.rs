//! Prints the snapshots and the updates of the order book of a symbol and a source until Enter is pressed, like
//! `IncOrderSnapshotSample` of the other dxFeed APIs. It builds an `IndexedTxModel` of orders on the default feed,
//! whose address is the `dxfeed.address` system property.
//!
//! Usage: `cargo run --example inc_order_snapshot -- [SYMBOL] [SOURCE] [ADDRESS]`, by default `AAPL` and `NTV`.
//! Without ADDRESS the address is `dxfeed.address` of the `dxfeed.system.properties` file in the current directory or
//! of the `DXFEED_dxfeed.address` environment variable, else `demo.dxfeed.com:7300`.

use std::process::ExitCode;

use dxfeed_graal::event::{EventFlags, Events, IndexedEventSource, Order, OrderSource};
use dxfeed_graal::model::IndexedTxModel;
use dxfeed_graal::{DXEndpoint, DXFeed, system};

/// How many orders of a call are printed; `0` prints them all.
const RECORD_PRINT_LIMIT: usize = 10;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let symbol = args.next().unwrap_or_else(|| "AAPL".into());
    let source = args.next().unwrap_or_else(|| "NTV".into());
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
    let source: IndexedEventSource = OrderSource::value_of(source)?;
    // The default feed connects to `dxfeed.address` when it is first used.
    system::set_property("dxfeed.address", &address)?;

    let printed_symbol = symbol.to_owned();
    let model = IndexedTxModel::<Order>::new_builder()
        .with_batch_processing(true) // All the pending transactions in one call.
        .with_snapshot_processing(true) // A snapshot as its resulting state.
        .with_feed(DXFeed::get_instance()?)
        .with_symbol(symbol)
        .with_sources([source])
        .with_listener(move |source, orders, is_snapshot| print(&printed_symbol, source, orders, is_snapshot))
        .build()?;
    println!("Subscribed to market depth updates for {symbol} from {address}; press Enter to stop.");

    // Enter or the end of input stops the program.
    let _ = std::io::stdin().read_line(&mut String::new());
    drop(model);
    DXEndpoint::get_instance()?.close_and_await_termination()
}

fn print(symbol: &str, source: IndexedEventSource, orders: Events<'_, Order>, is_snapshot: bool) {
    println!(
        "Snapshot {{ Symbol: '{symbol}', Source: '{source}', Type: {} }}",
        if is_snapshot { "full" } else { "update" }
    );
    let orders: Vec<_> = orders.collect();
    for (count, order) in orders.iter().enumerate().map(|(i, order)| (i + 1, order)) {
        let size = order.size();
        if !order.event_flags().contains(EventFlags::REMOVE_EVENT) && size != 0.0 && !size.is_nan() {
            println!("    {order}");
        } else {
            println!("    REMOVAL {{ Index: {:#x} }}", order.index());
        }
        if count < RECORD_PRINT_LIMIT || RECORD_PRINT_LIMIT == 0 {
            continue;
        }
        if orders.len() > count {
            println!("   {{ ... {} records left ...}}", orders.len() - count);
        }
        break;
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
