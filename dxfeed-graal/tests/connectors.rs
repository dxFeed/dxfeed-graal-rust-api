//! The message connectors of endpoints, with a local publisher as the peer.

use std::net::TcpListener;
use std::time::{Duration, Instant};

use dxfeed_graal::event::Quote;
use dxfeed_graal::{DXEndpoint, MessageConnector, MessageConnectorKind, MessageConnectorState, Role, State};

const TIMEOUT: Duration = Duration::from_secs(20);

/// A local port that was free a moment ago.
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// Polls `condition` until it holds, failing with `what` after [`TIMEOUT`].
fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + TIMEOUT;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// A publisher listening on a free port, and that port.
fn listening_publisher() -> (DXEndpoint, u16) {
    let port = free_port();
    let publisher = DXEndpoint::create_with_role(Role::Publisher).unwrap();
    publisher.connect(&format!(":{port}")).unwrap();
    wait_until("the publisher to listen", || {
        publisher.get_state().unwrap() == State::Connected
    });
    (publisher, port)
}

/// The only connector of `endpoint`.
fn only_connector(endpoint: &DXEndpoint) -> MessageConnector {
    let mut connectors = endpoint.get_connectors().unwrap();
    assert_eq!(connectors.len(), 1, "{connectors:?}");
    connectors.pop().unwrap()
}

#[test]
fn endpoints_without_a_connection_have_no_connectors() {
    let feed = DXEndpoint::create().unwrap();
    assert!(feed.get_connectors().unwrap().is_empty());

    let hub = DXEndpoint::create_with_role(Role::LocalHub).unwrap();
    assert!(hub.get_connectors().unwrap().is_empty());

    let closed = DXEndpoint::create().unwrap();
    closed.connect(&format!("127.0.0.1:{}", free_port())).unwrap();
    closed.close().unwrap();
    assert!(closed.get_connectors().unwrap().is_empty());
}

#[test]
fn a_feed_connected_to_a_publisher_has_a_connected_client_socket_connector() {
    let (publisher, port) = listening_publisher();
    let feed = DXEndpoint::create().unwrap();
    feed.connect(&format!("127.0.0.1:{port}")).unwrap();

    let connector = only_connector(&feed);
    wait_until("the client connector to connect", || {
        connector.get_state().unwrap() == MessageConnectorState::Connected
    });
    assert_eq!(connector.get_kind().unwrap(), MessageConnectorKind::ClientSocket);
    assert_eq!(connector.get_address().unwrap(), format!("127.0.0.1:{port}"));
    assert!(!connector.get_name().unwrap().is_empty());
    assert!(connector.is_active().unwrap());
    assert_eq!(connector.get_connection_count().unwrap(), 1);
    assert!(
        connector
            .get_current_addresses()
            .unwrap()
            .contains(&format!("127.0.0.1:{port}")),
        "{:?}",
        connector.get_current_addresses()
    );

    let server = only_connector(&publisher);
    assert_eq!(server.get_kind().unwrap(), MessageConnectorKind::ServerSocket);
    // QD names a server socket address with its bind address: `*` for all of them.
    assert_eq!(server.get_address().unwrap(), format!("*:{port}"));
    assert_eq!(server.get_state().unwrap(), MessageConnectorState::Connected);
    assert!(server.get_current_addresses().unwrap().is_empty());
    // The statistics may count the accepted connection a moment after the connection count.
    wait_until("the server connector to accept", || {
        server.get_connection_count().unwrap() == 1 && server.get_statistics().unwrap().active_connection_count == 1
    });
}

#[test]
fn an_address_with_two_hosts_gives_two_connectors() {
    let (first, second) = (free_port(), free_port());
    let feed = DXEndpoint::create().unwrap();
    feed.connect(&format!("(127.0.0.1:{first})(127.0.0.1:{second})"))
        .unwrap();
    let mut addresses: Vec<String> = feed
        .get_connectors()
        .unwrap()
        .iter()
        .map(|connector| connector.get_address().unwrap())
        .collect();
    addresses.sort();
    let mut expected = vec![format!("127.0.0.1:{first}"), format!("127.0.0.1:{second}")];
    expected.sort();
    assert_eq!(addresses, expected);
}

#[test]
fn the_address_hides_the_credentials() {
    let feed = DXEndpoint::create().unwrap();
    feed.connect(&format!("127.0.0.1:{}[user=rustuser,password=rustsecret]", free_port()))
        .unwrap();
    let address = only_connector(&feed).get_address().unwrap();
    assert!(address.starts_with("127.0.0.1:"), "{address}");
    assert!(!address.contains("rustsecret"), "{address}");
    assert!(!address.contains("rustuser"), "{address}");
}

#[test]
fn the_statistics_grow_with_the_data() {
    let (publisher, port) = listening_publisher();
    let feed = DXEndpoint::create().unwrap();
    let subscription = feed.get_feed().create_subscription::<Quote>().unwrap();
    subscription.add_symbols(["CONNECTORS"]).unwrap();
    feed.connect(&format!("127.0.0.1:{port}")).unwrap();
    let connector = only_connector(&feed);

    let first = connector.get_statistics().unwrap();
    // The publisher drops events until the subscription of the feed reaches it, so it publishes until they arrive.
    wait_until("data records", || {
        publisher
            .get_publisher()
            .publish_events(&[Quote::new("CONNECTORS").into()])
            .unwrap();
        connector.get_statistics().unwrap().read_data_records > 0
    });
    let second = connector.get_statistics().unwrap();
    assert!(second.read_bytes > first.read_bytes, "{first:?} {second:?}");
    assert!(
        second.read_data_records > first.read_data_records,
        "{first:?} {second:?}"
    );
    assert!(second.written_bytes >= first.written_bytes, "{first:?} {second:?}");
    assert!(
        second.written_data_records >= first.written_data_records,
        "{first:?} {second:?}"
    );
    assert_eq!(second.active_connection_count, 1);
    assert_eq!(second.closed_connection_count, 0);
    let third = connector.get_statistics().unwrap();
    assert!(third.read_bytes >= second.read_bytes && third.read_data_records >= second.read_data_records);
}

#[test]
fn a_connector_to_a_port_nobody_listens_on_is_connecting() {
    let feed = DXEndpoint::create().unwrap();
    feed.connect(&format!("127.0.0.1:{}", free_port())).unwrap();
    let connector = only_connector(&feed);
    assert_eq!(connector.get_state().unwrap(), MessageConnectorState::Connecting);
    assert!(connector.is_active().unwrap());
    assert_eq!(connector.get_connection_count().unwrap(), 0);
    assert!(connector.get_current_addresses().unwrap().is_empty());
}

#[test]
fn a_connect_to_another_address_replaces_the_connector() {
    let (publisher, first) = listening_publisher();
    let second = free_port();
    let feed = DXEndpoint::create().unwrap();
    feed.connect(&format!("127.0.0.1:{first}")).unwrap();
    let old = only_connector(&feed);
    wait_until("the client connector to connect", || {
        old.get_state().unwrap() == MessageConnectorState::Connected
    });
    wait_until("the first bytes read", || old.get_statistics().unwrap().read_bytes > 0);
    let name = old.get_name().unwrap();

    // A reconnect and a connect to the same address keep the connector: it stays started (QD connects it again after
    // its reconnect delay).
    feed.reconnect().unwrap();
    feed.connect(&format!("127.0.0.1:{first}")).unwrap();
    assert!(old.is_active().unwrap());
    assert_ne!(old.get_state().unwrap(), MessageConnectorState::Disconnected);
    let same = only_connector(&feed);
    assert_eq!(same.get_name().unwrap(), name);
    assert_eq!(same.get_address().unwrap(), format!("127.0.0.1:{first}"));

    feed.connect(&format!("127.0.0.1:{second}")).unwrap();
    assert_eq!(old.get_state().unwrap(), MessageConnectorState::Disconnected);
    assert!(!old.is_active().unwrap());
    assert_eq!(old.get_name().unwrap(), name);
    assert_eq!(old.get_kind().unwrap(), MessageConnectorKind::ClientSocket);
    assert_eq!(old.get_address().unwrap(), format!("127.0.0.1:{first}"));
    wait_until("the replaced connector to close its connection", || {
        old.get_connection_count().unwrap() == 0 && old.get_current_addresses().unwrap().is_empty()
    });
    // The statistics are kept: the counters never decrease.
    assert!(old.get_statistics().unwrap().read_bytes > 0);
    assert_eq!(
        only_connector(&feed).get_address().unwrap(),
        format!("127.0.0.1:{second}")
    );
    drop(publisher);
}

#[test]
fn a_disconnected_endpoint_has_no_connectors() {
    let feed = DXEndpoint::create().unwrap();
    feed.connect(&format!("127.0.0.1:{}", free_port())).unwrap();
    let connector = only_connector(&feed);
    feed.disconnect().unwrap();
    assert!(feed.get_connectors().unwrap().is_empty());
    assert_eq!(connector.get_state().unwrap(), MessageConnectorState::Disconnected);
    assert!(!connector.is_active().unwrap());
}

#[test]
fn a_connector_answers_after_its_endpoint_is_closed_and_dropped() {
    let (publisher, port) = listening_publisher();
    let feed = DXEndpoint::create().unwrap();
    feed.connect(&format!("127.0.0.1:{port}")).unwrap();
    let connector = only_connector(&feed);
    wait_until("the client connector to connect", || {
        connector.get_state().unwrap() == MessageConnectorState::Connected
    });
    wait_until("the first bytes read", || {
        connector.get_statistics().unwrap().read_bytes > 0
    });
    let name = connector.get_name().unwrap();
    let before = connector.get_statistics().unwrap();
    feed.close().unwrap();
    drop(feed);

    assert_eq!(connector.get_name().unwrap(), name);
    assert_eq!(connector.get_kind().unwrap(), MessageConnectorKind::ClientSocket);
    assert_eq!(connector.get_address().unwrap(), format!("127.0.0.1:{port}"));
    assert_eq!(connector.get_state().unwrap(), MessageConnectorState::Disconnected);
    assert!(!connector.is_active().unwrap());
    // The connection may close a moment after the endpoint.
    wait_until("the connection to close", || {
        connector.get_connection_count().unwrap() == 0
            && connector.get_current_addresses().unwrap().is_empty()
            && connector.get_closed_connection_count().unwrap() >= 1
            && connector.get_statistics().unwrap().active_connection_count == 0
    });
    let statistics = connector.get_statistics().unwrap();
    assert!(statistics.read_bytes >= before.read_bytes, "{before:?} {statistics:?}");
    assert!(
        statistics.written_bytes >= before.written_bytes,
        "{before:?} {statistics:?}"
    );
    drop(publisher);
}

#[test]
fn a_connector_is_shared_with_other_threads() {
    let feed = DXEndpoint::create().unwrap();
    feed.connect(&format!("127.0.0.1:{}", free_port())).unwrap();
    let connector = only_connector(&feed);
    let name = connector.get_name().unwrap();
    let clone = connector.clone();
    let moved = std::thread::spawn(move || clone.get_name().unwrap()).join().unwrap();
    assert_eq!(moved, name);
    assert_eq!(format!("{connector:?}"), "MessageConnector { .. }");
}

#[test]
fn kinds_and_states_print_as_the_sdk_constants() {
    let kinds: Vec<String> = [
        MessageConnectorKind::ClientSocket,
        MessageConnectorKind::ServerSocket,
        MessageConnectorKind::Other,
    ]
    .iter()
    .map(MessageConnectorKind::to_string)
    .collect();
    assert_eq!(kinds, ["CLIENT_SOCKET", "SERVER_SOCKET", "OTHER"]);
    let states: Vec<String> = [
        MessageConnectorState::Connecting,
        MessageConnectorState::Connected,
        MessageConnectorState::Disconnected,
    ]
    .iter()
    .map(MessageConnectorState::to_string)
    .collect();
    assert_eq!(states, ["CONNECTING", "CONNECTED", "DISCONNECTED"]);
}
