//! Converts one tape file into another, passing every event through a listener that may change or filter it, like
//! `ConvertTapeFileSample` of the other dxFeed APIs. The `write_tape_file` sample writes a tape file to convert.
//!
//! Usage: `cargo run --example ConvertTapeFileSample -- [INPUT] [OUTPUT]`, by default
//! `file:ConvertTapeFile.in[readAs=stream_data,speed=max]` and
//! `tape:ConvertTapeFile.out[saveAs=stream_data,format=text]`.

use std::process::ExitCode;

use dxfeed_graal::{DXEndpoint, Event, EventType, Role, Symbol};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let input = args
        .next()
        .unwrap_or_else(|| "file:ConvertTapeFile.in[readAs=stream_data,speed=max]".into());
    let output = args
        .next()
        .unwrap_or_else(|| "tape:ConvertTapeFile.out[saveAs=stream_data,format=text]".into());
    match run(&input, &output) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(input: &str, output: &str) -> dxfeed_graal::Result<()> {
    // A stream feed neither conflates nor drops events; the wildcard subscribes to every symbol, and the events keep
    // the times of the file.
    let input_endpoint = DXEndpoint::new_builder()
        .with_role(Role::StreamFeed)
        .with_property("dxfeed.wildcard.enable", "true")
        .with_property("dxendpoint.eventTime", "true")
        .build()?;
    // A stream publisher writes every event it publishes.
    let output_endpoint = DXEndpoint::new_builder()
        .with_role(Role::StreamPublisher)
        .with_property("dxfeed.wildcard.enable", "true")
        .with_property("dxendpoint.eventTime", "true")
        .build()?;

    // The listener of all the event types copies the events to the output; here they could be changed, removed or
    // added, for example the event times moved by an hour.
    let subscription = input_endpoint
        .get_feed()
        .create_subscription_with_types(EventType::ALL)?;
    let publisher = output_endpoint.get_publisher();
    subscription.add_event_listener(move |events| {
        let events: Vec<Event> = events.filter_map(|event| event.to_owned()).collect();
        if let Err(error) = publisher.publish_events(&events) {
            eprintln!("error: {error}");
        }
    })?;
    subscription.add_symbols([Symbol::Wildcard])?;

    // The output starts writing before the input starts reading.
    output_endpoint.connect(output)?;
    input_endpoint.connect(input)?;

    // The input disconnects when the file is read; closing waits for the listener.
    input_endpoint.await_not_connected()?;
    input_endpoint.close_and_await_termination()?;
    // The output writes the events it has, then closes.
    output_endpoint.await_processed()?;
    output_endpoint.close_and_await_termination()
}
