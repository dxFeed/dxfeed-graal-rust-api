//! The event printing and the tape of the event tools, shared by Connect and Dump.

use std::io::Write;

use dxfeed_graal::{DXEndpoint, DXFeedSubscription, Event, Role};

/// Prints every event of the subscription in the format of Java's `toString`, one per line.
///
/// A closed standard output, such as a pipe whose reader is gone, ends the process, as the broken pipe signal ends the
/// C++ tools; otherwise a tool that runs until it is stopped would go on with nobody reading it.
pub fn print_events(subscription: &DXFeedSubscription<Event>) -> dxfeed_graal::Result<()> {
    subscription.add_event_listener(|mut events| {
        let mut out = std::io::stdout().lock();
        let written = events
            .try_for_each(|event| writeln!(out, "{event}"))
            .and_then(|()| out.flush());
        if written.is_err() {
            std::process::exit(0);
        }
    })?;
    Ok(())
}

/// Publishes every event of the subscription to the tape `tape`, with `tape:` added if it lacks it, through a
/// `StreamPublisher` endpoint named `<tool>Tool-Publisher` with the wildcard enabled and `properties`; returns the
/// endpoint.
///
/// The events of a type this crate does not know ([`EventRef::Other`](dxfeed_graal::EventRef::Other)) are not taped.
/// When the publisher refuses a batch, its events are published one by one, so that one refused event loses only
/// itself; each refusal is printed to the standard error.
pub fn tape(
    tool: &str,
    tape: &str,
    properties: &[(String, String)],
    subscription: &DXFeedSubscription<Event>,
) -> dxfeed_graal::Result<DXEndpoint> {
    let endpoint = DXEndpoint::new_builder()
        .with_role(Role::StreamPublisher)
        .with_property(DXEndpoint::DXFEED_WILDCARD_ENABLE_PROPERTY, "true")
        .with_properties(properties.iter().map(|(key, value)| (key, value)))
        .with_name(&format!("{tool}Tool-Publisher"))
        .build()?;
    if tape.starts_with("tape:") {
        endpoint.connect(tape)?;
    } else {
        endpoint.connect(&format!("tape:{tape}"))?;
    }
    let publisher = endpoint.get_publisher();
    subscription.add_event_listener(move |events| {
        let events: Vec<Event> = events.filter_map(|event| event.to_owned()).collect();
        if publisher.publish_events(&events).is_err() {
            for event in &events {
                if let Err(error) = publisher.publish_events(std::slice::from_ref(event)) {
                    eprintln!("{error}");
                }
            }
        }
    })?;
    Ok(endpoint)
}
