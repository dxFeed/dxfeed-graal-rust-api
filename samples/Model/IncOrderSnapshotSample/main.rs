//! Usage: `cargo run --bin IncOrderSnapshotSample`

use dxfeed_graal::event::{EventFlags, Events, IndexedEventSource, Order, OrderSource};
use dxfeed_graal::model::IndexedTxModel;
use dxfeed_graal::{DXEndpoint, DXFeed};

fn main() -> dxfeed_graal::Result<()> {
    let address = "demo.dxfeed.com:7300";
    let symbol = "AAPL";
    let record_print_limit = 10_usize;

    let event_printer = move |source: IndexedEventSource, events: Events<'_, Order>, is_snapshot: bool| {
        println!(
            "Snapshot {{ Symbol: '{}', Source: '{}', Type: {} }}",
            symbol,
            source,
            if is_snapshot { "full" } else { "update" }
        );

        let events: Vec<_> = events.collect();
        let mut count: usize = 0;

        for order in &events {
            // `hasSize`: the size is neither zero nor NaN.
            let has_size = order.size() != 0.0 && !order.size().is_nan();

            if !order.event_flags().contains(EventFlags::REMOVE_EVENT) && has_size {
                println!("    {order}");
            } else {
                println!("    REMOVAL {{ Index: {:#x} }}", order.index());
            }

            count += 1;

            if count < record_print_limit || record_print_limit == 0 {
                continue;
            }

            if events.len() - count > 0 {
                println!("   {{ ... {} records left ...}}", events.len() - count);
            }

            break;
        }
    };

    // Build the model with the specified configurations.
    let _model = IndexedTxModel::<Order>::new_builder()
        .with_batch_processing(true) // Enable batch processing.
        .with_snapshot_processing(true) // Enable snapshot processing.
        .with_feed(DXFeed::get_instance()?) // Attach the model to the DXFeed instance.
        .with_symbol(symbol) // Set the symbol to subscribe to.
        .with_sources([OrderSource::NTV]) // Set the source for indexed events.
        .with_listener(event_printer)
        .build()?;

    DXEndpoint::get_instance()?.connect(address)?;

    println!("Subscribed to market depth updates for {symbol}. Press [ENTER] to exit.");

    let _ = std::io::stdin().read_line(&mut String::new());
    Ok(())
}
