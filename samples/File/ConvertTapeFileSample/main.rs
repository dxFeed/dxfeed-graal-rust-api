//! Converts one tape file into another tape file with optional intermediate processing or filtering.
//! This project contains a sample tape file `ConvertTapeFile.in`.
//!
//! Usage: `cargo run --bin ConvertTapeFileSample -- [<inputAddress> [<outputAddress>]]`, run from the sample
//! directory when the default input file `ConvertTapeFile.in` is used.

use dxfeed_graal::{DXEndpoint, Event, EventType, Role, Symbol};

fn main() -> dxfeed_graal::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    // Determine input and output tapes and specify appropriate configuration parameters.
    let input_address = if args.len() > 1 {
        args[1].clone()
    } else {
        "file:ConvertTapeFile.in[readAs=stream_data,speed=max]".to_owned()
    };
    let output_address = if args.len() > 2 {
        args[2].clone()
    } else {
        "tape:ConvertTapeFile.out[saveAs=stream_data,format=text]".to_owned()
    };

    // Create an input endpoint configured for tape reading.
    let input_endpoint = DXEndpoint::new_builder()
        .with_role(Role::StreamFeed) // Prevents event conflation and loss due to buffer overflow.
        .with_property("dxfeed.wildcard.enable", "true") // Enables wildcard subscription.
        .with_property("dxendpoint.eventTime", "true") // Use provided event times.
        .build()?;

    // Create an output endpoint configured for tape writing.
    let output_endpoint = DXEndpoint::new_builder()
        .with_role(Role::StreamPublisher) // Prevents event conflation and loss due to buffer overflow.
        .with_property("dxfeed.wildcard.enable", "true") // Enables wildcard subscription.
        .with_property("dxendpoint.eventTime", "true") // Use provided event times.
        .build()?;

    // Create and link event processor for all types of events.
    // Note: Set of processed event types could be limited if needed.
    let sub = input_endpoint
        .get_feed()
        .create_subscription_with_types(EventType::ALL)?;

    let output = output_endpoint.clone();

    sub.add_event_listener(move |events| {
        // Here event processing occurs. Events could be modified, removed, or new events added.
        // For example, the event times could be moved by 1 hour: match every `Event` variant and add 3_600_000 to the
        // `event_time` field of its struct (there is no setter common for all the event types).
        let events: Vec<Event> = events.filter_map(|event| event.to_owned()).collect();

        // Publish processed events
        let publisher = output.get_publisher();

        if let Err(e) = publisher.publish_events(&events) {
            eprintln!("{e}");
        }
    })?;

    // Subscribe to all symbols.
    // Note: Set of processed symbols could be limited if needed.
    sub.add_symbols([Symbol::Wildcard])?;

    // Connect output endpoint and start output tape writing BEFORE starting input tape reading.
    output_endpoint.connect(&output_address)?;
    // Connect input endpoint and start input tape reading AFTER starting output tape writing.
    input_endpoint.connect(&input_address)?;

    // Wait until all data is read and processed, and then gracefully close the input endpoint.
    input_endpoint.await_not_connected()?;
    input_endpoint.close_and_await_termination()?;

    // Wait until all data is processed and written, and then gracefully close the output endpoint.
    output_endpoint.await_processed()?;
    output_endpoint.close_and_await_termination()?;

    Ok(())
}
