//! Endpoint states, state listeners and the lifecycle calls, with a local publisher as the peer and tape files.

use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use dxfeed_graal::event::Quote;
use dxfeed_graal::{DXEndpoint, Error, Event, Role, State};

const TIMEOUT: Duration = Duration::from_secs(20);

/// A local port that was free a moment ago.
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// A path in the temporary directory, unique to this process and `name`.
fn temp_file(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("dxfeed-graal-{}-{name}", std::process::id()))
}

/// Polls the state of `endpoint` until it is `expected`, failing after [`TIMEOUT`].
fn await_state(endpoint: &DXEndpoint, expected: State) {
    let deadline = Instant::now() + TIMEOUT;
    while endpoint.state().unwrap() != expected {
        assert!(Instant::now() < deadline, "no {expected} state");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Receives states until `expected` arrives, failing after [`TIMEOUT`].
fn wait_for(states: &Receiver<(State, State)>, expected: State) {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match states.recv_timeout(left) {
            Ok((_, new)) if new == expected => return,
            Ok(_) => {}
            Err(e) => panic!("no {expected} state: {e}"),
        }
    }
}

#[test]
fn a_new_endpoint_is_not_connected_and_a_closed_one_is_closed() {
    let endpoint = DXEndpoint::create().unwrap();
    assert_eq!(endpoint.state().unwrap(), State::NotConnected);
    // Neither call needs a connection.
    endpoint.reconnect().unwrap();
    endpoint.disconnect_and_clear().unwrap();
    assert_eq!(endpoint.state().unwrap(), State::NotConnected);
    endpoint.close().unwrap();
    assert_eq!(endpoint.state().unwrap(), State::Closed);
}

#[test]
fn states_print_as_the_java_constants() {
    let printed: Vec<String> = [State::NotConnected, State::Connecting, State::Connected, State::Closed]
        .iter()
        .map(State::to_string)
        .collect();
    assert_eq!(printed, ["NOT_CONNECTED", "CONNECTING", "CONNECTED", "CLOSED"]);
}

#[test]
fn the_listener_sees_the_connection_to_a_peer_and_the_close() {
    let port = free_port();
    let publisher = DXEndpoint::create_with_role(Role::Publisher).unwrap();
    publisher.connect(&format!(":{port}")).unwrap();
    // Listening, so the client does not have to wait for a reconnect, and a port taken meanwhile fails here.
    await_state(&publisher, State::Connected);

    let endpoint = DXEndpoint::create().unwrap();
    let (sender, states) = mpsc::channel();
    endpoint
        .add_state_listener(move |old, new| {
            let _ = sender.send((old, new));
        })
        .unwrap();
    endpoint.connect(&format!("127.0.0.1:{port}")).unwrap();
    wait_for(&states, State::Connected);
    assert_eq!(endpoint.state().unwrap(), State::Connected);

    endpoint.close().unwrap();
    wait_for(&states, State::Closed);
}

#[test]
fn a_removed_listener_is_not_called_and_an_unknown_one_is_an_error() {
    let endpoint = DXEndpoint::create().unwrap();
    let (sender, states) = mpsc::channel();
    let id = endpoint
        .add_state_listener(move |old, new| {
            let _ = sender.send((old, new));
        })
        .unwrap();
    endpoint.remove_state_listener(id).unwrap();
    assert!(matches!(
        endpoint.remove_state_listener(id),
        Err(Error::InvalidArgument(_))
    ));
    endpoint.close_and_await_termination().unwrap();
    // The sender was dropped with the listener, so nothing more can arrive.
    assert!(states.recv_timeout(Duration::from_secs(1)).is_err());
}

#[test]
fn a_panicking_listener_keeps_being_called() {
    let endpoint = DXEndpoint::create().unwrap();
    let (sender, states) = mpsc::channel();
    endpoint
        .add_state_listener(move |old, new| {
            let _ = sender.send((old, new));
            panic!("test panic in a state listener");
        })
        .unwrap();
    // Nothing listens on the port, so the endpoint keeps connecting.
    endpoint.connect(&format!("127.0.0.1:{}", free_port())).unwrap();
    wait_for(&states, State::Connecting);
    // The change after the panicking call arrives too.
    endpoint.close().unwrap();
    wait_for(&states, State::Closed);
}

#[test]
fn a_tape_file_written_by_a_publisher_is_read_back() {
    let file = temp_file("quotes.txt");
    let _ = std::fs::remove_file(&file);
    let sent = [
        Quote {
            bid_price: 10.1,
            ask_price: 10.2,
            ..Quote::new("TEST1")
        },
        Quote {
            bid_price: 17.1,
            ask_price: 17.2,
            ..Quote::new("TEST2")
        },
    ];

    // Without a subscription a publisher drops its events; the wildcard subscribes the tape to everything.
    let writer = DXEndpoint::new_builder()
        .with_role(Role::Publisher)
        .with_property("dxfeed.wildcard.enable", "true")
        .build()
        .unwrap();
    writer
        .connect(&format!("tape:{}[format=text]", file.display()))
        .unwrap();
    let events: Vec<Event> = sent.iter().cloned().map(Event::from).collect();
    writer.publisher().publish_events(&events).unwrap();
    writer.await_processed().unwrap();
    writer.close_and_await_termination().unwrap();

    let reader = DXEndpoint::create_with_role(Role::StreamFeed).unwrap();
    let subscription = reader.feed().create_subscription::<Quote>().unwrap();
    let (sender, received) = mpsc::channel();
    subscription
        .add_event_listener(move |quotes| {
            for quote in quotes {
                let _ = sender.send(quote.to_owned());
            }
        })
        .unwrap();
    subscription.add_symbols(["TEST1", "TEST2"]).unwrap();
    reader.connect(&format!("file:{}[speed=max]", file.display())).unwrap();
    reader.await_not_connected().unwrap();
    reader.close_and_await_termination().unwrap();
    drop(subscription);

    let mut quotes: Vec<(String, f64, f64)> = received
        .try_iter()
        .map(|q| (q.symbol, q.bid_price, q.ask_price))
        .collect();
    quotes.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(quotes, [("TEST1".into(), 10.1, 10.2), ("TEST2".into(), 17.1, 17.2)]);
    let _ = std::fs::remove_file(&file);
}

#[test]
fn the_waits_refuse_to_run_inside_a_listener() {
    let hub = DXEndpoint::create_with_role(Role::LocalHub).unwrap();
    let subscription = hub.feed().create_subscription::<Quote>().unwrap();
    let (sender, results) = mpsc::channel();
    let inner = hub.clone();
    subscription
        .add_event_listener(move |_| {
            let _ = sender.send((inner.await_processed(), inner.await_not_connected()));
        })
        .unwrap();
    subscription.add_symbols(["IBM"]).unwrap();
    hub.publisher().publish_events(&[Quote::new("IBM").into()]).unwrap();
    let (processed, not_connected) = results.recv_timeout(TIMEOUT).unwrap();
    assert!(matches!(processed, Err(Error::InCallback)));
    assert!(matches!(not_connected, Err(Error::InCallback)));
}
