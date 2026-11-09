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

/// The experimental connector listener and connection errors.
#[cfg(feature = "experimental")]
mod experimental {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::mpsc::{self, RecvTimeoutError};
    use std::sync::{Arc, Mutex, OnceLock};
    use std::time::{Duration, Instant};

    use dxfeed_graal::{
        ConnectionError, ConnectionErrorCategory, DXEndpoint, Error, ErrorManager, MessageConnector,
        MessageConnectorState,
    };

    use super::{TIMEOUT, free_port, listening_publisher, only_connector, wait_until};

    /// Waits until `connector` has an error with at least `count` errors.
    fn wait_for_error(connector: &MessageConnector, count: i64) -> ConnectionError {
        let mut last = None;
        wait_until("a connection error", || {
            last = connector.get_last_error().unwrap();
            last.as_ref().is_some_and(|error| error.count >= count)
        });
        last.unwrap()
    }

    /// Whether the sender of `receiver` is dropped within [`TIMEOUT`], ignoring what it still sends.
    fn sender_dropped<T>(receiver: &mpsc::Receiver<T>) -> bool {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(_) => continue,
                Err(RecvTimeoutError::Disconnected) => return true,
                Err(RecvTimeoutError::Timeout) => return false,
            }
        }
    }

    #[test]
    fn a_listener_sees_a_connector_connect() {
        let (publisher, port) = listening_publisher();
        let feed = DXEndpoint::create().unwrap();
        let (sender, states) = mpsc::channel();
        feed.add_message_connector_listener(move |connector| {
            let _ = sender.send((connector.get_address().unwrap(), connector.get_state().unwrap()));
        })
        .unwrap();
        feed.connect(&format!("127.0.0.1:{port}")).unwrap();
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let (address, state) = states
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("no CONNECTED call");
            assert_eq!(address, format!("127.0.0.1:{port}"));
            if state == MessageConnectorState::Connected {
                break;
            }
        }
        drop(publisher);
    }

    #[test]
    fn no_call_runs_or_follows_once_a_listener_is_removed() {
        let (publisher, port) = listening_publisher();
        let feed = DXEndpoint::create().unwrap();
        let in_call = Arc::new(AtomicBool::new(false));
        let calls = Arc::new(AtomicUsize::new(0));
        let (sender, entered) = mpsc::channel();
        let id = {
            let (in_call, calls) = (in_call.clone(), calls.clone());
            feed.add_message_connector_listener(move |_| {
                in_call.store(true, Ordering::SeqCst);
                calls.fetch_add(1, Ordering::SeqCst);
                let _ = sender.send(());
                std::thread::sleep(Duration::from_millis(300));
                in_call.store(false, Ordering::SeqCst);
            })
            .unwrap()
        };
        feed.connect(&format!("127.0.0.1:{port}")).unwrap();
        entered.recv_timeout(TIMEOUT).expect("no call");
        // Removing waits for the running call.
        feed.remove_message_connector_listener(id).unwrap();
        assert!(!in_call.load(Ordering::SeqCst));
        let seen = calls.load(Ordering::SeqCst);
        assert!(sender_dropped(&entered));
        // New changes call nothing.
        feed.connect(&format!("127.0.0.1:{}", free_port())).unwrap();
        feed.disconnect().unwrap();
        std::thread::sleep(Duration::from_millis(500));
        assert_eq!(calls.load(Ordering::SeqCst), seen);
        assert!(matches!(
            feed.remove_message_connector_listener(id),
            Err(Error::InvalidArgument(_))
        ));
        drop(publisher);
    }

    #[test]
    fn a_panicking_listener_keeps_being_called() {
        let feed = DXEndpoint::create().unwrap();
        let (sender, calls) = mpsc::channel();
        feed.add_message_connector_listener(move |_| {
            let _ = sender.send(());
            panic!("test panic in a connector listener");
        })
        .unwrap();
        feed.connect(&format!("127.0.0.1:{}", free_port())).unwrap();
        calls.recv_timeout(TIMEOUT).expect("no first call");
        // The disconnect after the panicking call is a change too.
        feed.disconnect().unwrap();
        calls.recv_timeout(TIMEOUT).expect("no call after the panic");
        wait_until("the panic report", || {
            ErrorManager::get_errors().iter().any(|error| {
                error.component == "DXEndpoint" && error.message.contains("test panic in a connector listener")
            })
        });
    }

    #[test]
    fn calls_inside_the_listener_work_and_it_removes_itself_at_once() {
        let (publisher, port) = listening_publisher();
        let feed = DXEndpoint::create().unwrap();
        let id = Arc::new(OnceLock::new());
        let (sender, results) = mpsc::channel();
        let listener_id = {
            let (inner, id) = (feed.clone(), id.clone());
            feed.add_message_connector_listener(move |connector| {
                let started = Instant::now();
                let connectors = inner.get_connectors().map(|connectors| connectors.len());
                let name = connector.get_name();
                let error = connector.get_last_error();
                let awaited = inner.await_processed();
                let removed = inner.remove_message_connector_listener(*id.get().unwrap());
                let _ = sender.send((
                    connectors.is_ok(),
                    name.is_ok(),
                    error.is_ok(),
                    matches!(awaited, Err(Error::InCallback)),
                    removed.is_ok(),
                    started.elapsed(),
                ));
            })
            .unwrap()
        };
        id.set(listener_id).unwrap();
        feed.connect(&format!("127.0.0.1:{port}")).unwrap();
        let (connectors, name, error, awaited, removed, elapsed) = results.recv_timeout(TIMEOUT).expect("no call");
        assert!(connectors && name && error && awaited && removed);
        assert!(elapsed < Duration::from_secs(5), "{elapsed:?}");
        // The listener is gone: its closure, with the sender, is released.
        assert!(sender_dropped(&results));
        drop(publisher);
    }

    #[test]
    fn the_listener_disconnects_and_closes_its_endpoint() {
        let (publisher, port) = listening_publisher();
        let feed = DXEndpoint::create().unwrap();
        let (sender, results) = mpsc::channel();
        let id = {
            let inner = feed.clone();
            feed.add_message_connector_listener(move |connector| match connector.get_state() {
                Ok(MessageConnectorState::Connected) => {
                    let _ = sender.send(("disconnect", inner.disconnect().is_ok()));
                }
                Ok(MessageConnectorState::Disconnected) if inner.get_connectors().is_ok_and(|c| c.is_empty()) => {
                    let _ = sender.send(("close", inner.close().is_ok()));
                }
                _ => {}
            })
            .unwrap()
        };
        feed.connect(&format!("127.0.0.1:{port}")).unwrap();
        assert_eq!(results.recv_timeout(TIMEOUT), Ok(("disconnect", true)));
        assert_eq!(results.recv_timeout(TIMEOUT), Ok(("close", true)));
        wait_until("the endpoint to close", || {
            feed.get_state().unwrap() == dxfeed_graal::State::Closed
        });
        // The closure owns a clone of the endpoint; removing the listener lets it go.
        feed.remove_message_connector_listener(id).unwrap();
        drop(publisher);
    }

    #[test]
    fn dropping_the_last_endpoint_inside_the_listener_returns() {
        let (publisher, port) = listening_publisher();
        let feed = DXEndpoint::create().unwrap();
        let holder = Arc::new(Mutex::new(None::<DXEndpoint>));
        let (sender, dropped) = mpsc::channel();
        {
            let holder = holder.clone();
            feed.add_message_connector_listener(move |_| {
                let endpoint = holder.lock().unwrap().take();
                if let Some(endpoint) = endpoint {
                    drop(endpoint);
                    let _ = sender.send(());
                }
            })
            .unwrap();
        }
        // The holder gets the only owner; the listener, called after the connect returns, drops it.
        let mut held = holder.lock().unwrap();
        held.insert(feed).connect(&format!("127.0.0.1:{port}")).unwrap();
        drop(held);
        dropped
            .recv_timeout(TIMEOUT)
            .expect("the drop inside the listener did not return");
        drop(publisher);
    }

    #[test]
    fn dropping_the_endpoint_removes_its_listeners() {
        let feed = DXEndpoint::create().unwrap();
        let (sender, calls) = mpsc::channel::<()>();
        feed.add_message_connector_listener(move |_| {
            let _ = sender.send(());
        })
        .unwrap();
        feed.connect(&format!("127.0.0.1:{}", free_port())).unwrap();
        drop(feed);
        assert!(sender_dropped(&calls));
    }

    #[test]
    fn a_listener_added_to_a_closed_endpoint_is_never_called() {
        let feed = DXEndpoint::create().unwrap();
        feed.close().unwrap();
        let (sender, calls) = mpsc::channel::<()>();
        let id = feed
            .add_message_connector_listener(move |_| {
                let _ = sender.send(());
            })
            .unwrap();
        assert!(feed.connect(&format!("127.0.0.1:{}", free_port())).is_ok());
        assert!(feed.get_connectors().unwrap().is_empty());
        assert_eq!(
            calls.recv_timeout(Duration::from_millis(500)),
            Err(RecvTimeoutError::Timeout)
        );
        feed.remove_message_connector_listener(id).unwrap();
        assert!(sender_dropped(&calls));
    }

    #[test]
    fn a_failed_connect_is_a_connect_failed_error() {
        let feed = DXEndpoint::create().unwrap();
        assert_eq!(feed.get_last_connection_error().unwrap(), None);
        let address = format!("127.0.0.1:{}", free_port());
        feed.connect(&address).unwrap();
        let connector = only_connector(&feed);
        let error = wait_for_error(&connector, 1);
        assert_eq!(error.category, ConnectionErrorCategory::ConnectFailed, "{error:?}");
        assert!(!error.dropped, "{error:?}");
        assert_eq!(error.exception_class_name.as_deref(), Some("java.net.ConnectException"));
        assert_eq!(error.address, address);
        assert!(error.time > 0);
        let endpoint_error = feed.get_last_connection_error().unwrap().expect("no endpoint error");
        assert_eq!(endpoint_error.category, ConnectionErrorCategory::ConnectFailed);
        assert_eq!(endpoint_error.address, address);
    }

    #[test]
    fn a_connector_keeps_its_error_after_the_endpoint_is_closed() {
        let feed = DXEndpoint::create().unwrap();
        feed.connect(&format!("127.0.0.1:{}", free_port())).unwrap();
        let connector = only_connector(&feed);
        let error = wait_for_error(&connector, 1);
        feed.close().unwrap();
        assert!(feed.get_last_connection_error().unwrap().is_some());
        drop(feed);
        let kept = connector.get_last_error().unwrap().expect("no error after the close");
        assert!(kept.count >= error.count);
        assert_eq!(kept.category, ConnectionErrorCategory::ConnectFailed);
    }

    #[test]
    fn a_connection_closed_by_the_publisher_is_a_dropped_error() {
        let (publisher, port) = listening_publisher();
        let feed = DXEndpoint::create().unwrap();
        feed.connect(&format!("127.0.0.1:{port}")).unwrap();
        let connector = only_connector(&feed);
        wait_until("the connector to connect", || {
            connector.get_state().unwrap() == MessageConnectorState::Connected
        });
        assert_eq!(connector.get_last_error().unwrap(), None);
        publisher.close().unwrap();
        let mut last = None;
        wait_until("a dropped error", || {
            last = connector.get_last_error().unwrap();
            last.as_ref().is_some_and(|error| error.dropped)
        });
        let error = last.unwrap();
        assert!(
            matches!(
                error.category,
                ConnectionErrorCategory::ClosedByRemote | ConnectionErrorCategory::Io
            ),
            "{error:?}"
        );
    }

    #[test]
    fn a_silent_peer_gives_a_heartbeat_timeout() {
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = server.local_addr().unwrap().port();
        // Accepts the connections and keeps them open without sending a byte.
        std::thread::spawn(move || {
            let mut streams = Vec::new();
            for stream in server.incoming() {
                streams.push(stream);
            }
        });
        let feed = DXEndpoint::create().unwrap();
        feed.connect(&format!("127.0.0.1:{port}[heartbeatTimeout=1]")).unwrap();
        let connector = only_connector(&feed);
        let mut last = None;
        wait_until("a heartbeat timeout", || {
            last = connector.get_last_error().unwrap();
            last.as_ref()
                .is_some_and(|error| error.category == ConnectionErrorCategory::HeartbeatTimeout)
        });
        let error = last.unwrap();
        assert!(error.dropped, "{error:?}");
        assert_eq!(error.exception_class_name, None);
    }

    #[test]
    fn an_error_hides_the_credentials() {
        let feed = DXEndpoint::create().unwrap();
        feed.connect(&format!("127.0.0.1:{}[user=rustuser,password=rustsecret]", free_port()))
            .unwrap();
        let error = wait_for_error(&only_connector(&feed), 1);
        assert!(error.address.starts_with("127.0.0.1:"), "{error:?}");
        let text = format!("{} {:?}", error.address, error.message);
        assert!(!text.contains("rustsecret"), "{text}");
        assert!(!text.contains("rustuser"), "{text}");
    }

    #[test]
    fn a_later_success_keeps_the_last_error() {
        let feed = DXEndpoint::create().unwrap();
        feed.connect(&format!("127.0.0.1:{}", free_port())).unwrap();
        let failed = only_connector(&feed);
        let error = wait_for_error(&failed, 1);
        let (publisher, port) = listening_publisher();
        feed.connect(&format!("127.0.0.1:{port}")).unwrap();
        let connector = only_connector(&feed);
        wait_until("the connector to connect", || {
            connector.get_state().unwrap() == MessageConnectorState::Connected
        });
        assert_eq!(connector.get_last_error().unwrap(), None);
        let kept = feed.get_last_connection_error().unwrap().expect("the error is gone");
        assert_eq!(kept.address, error.address);
        assert!(failed.get_last_error().unwrap().is_some());
        drop(publisher);
    }

    #[test]
    fn the_error_is_recorded_before_the_listener_is_called() {
        let feed = DXEndpoint::create().unwrap();
        let (sender, errors) = mpsc::channel();
        feed.add_message_connector_listener(move |connector| {
            let _ = sender.send(connector.get_last_error().unwrap());
        })
        .unwrap();
        feed.connect(&format!("127.0.0.1:{}", free_port())).unwrap();
        let deadline = Instant::now() + TIMEOUT;
        // The calls before the failure see no error; the call of the first failure sees it, not only a later one.
        loop {
            let error = errors
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("no call with an error");
            if let Some(error) = error {
                assert_eq!(error.category, ConnectionErrorCategory::ConnectFailed);
                assert_eq!(error.count, 1, "{error:?}");
                break;
            }
        }
    }

    #[test]
    fn categories_print_as_the_sdk_constants() {
        let categories: Vec<String> = [
            ConnectionErrorCategory::Other,
            ConnectionErrorCategory::Dns,
            ConnectionErrorCategory::ConnectFailed,
            ConnectionErrorCategory::ReadTimeout,
            ConnectionErrorCategory::HeartbeatTimeout,
            ConnectionErrorCategory::ClosedByRemote,
            ConnectionErrorCategory::Tls,
            ConnectionErrorCategory::Io,
        ]
        .iter()
        .map(ConnectionErrorCategory::to_string)
        .collect();
        assert_eq!(
            categories,
            [
                "OTHER",
                "DNS",
                "CONNECT_FAILED",
                "READ_TIMEOUT",
                "HEARTBEAT_TIMEOUT",
                "CLOSED_BY_REMOTE",
                "TLS",
                "IO"
            ]
        );
    }
}
