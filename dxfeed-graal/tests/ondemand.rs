//! The on-demand service without a provider of historical data: its modes, the errors of the replay and the property
//! change listeners; no network needed.

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use dxfeed_graal::ondemand::{OnDemandService, PropertyValue};
use dxfeed_graal::{DXEndpoint, Role};

type Change = (Option<String>, PropertyValue, PropertyValue, thread::ThreadId);

/// A listener that sends each change with the thread that got it.
fn listener(sender: mpsc::Sender<Change>) -> impl FnMut(Option<&str>, PropertyValue, PropertyValue) + Send + 'static {
    move |name, old, new| {
        let _ = sender.send((name.map(str::to_owned), old, new, thread::current().id()));
    }
}

/// Waits for the change of `name`, skipping the changes of other properties.
fn change_of(receiver: &mpsc::Receiver<Change>, name: &str) -> Change {
    loop {
        let change = receiver.recv_timeout(Duration::from_secs(5)).expect(name);
        if change.0.as_deref() == Some(name) {
            return change;
        }
    }
}

#[test]
fn the_default_service_has_its_own_endpoint() {
    let service = OnDemandService::get_instance().unwrap();
    assert!(std::ptr::eq(service, OnDemandService::get_instance().unwrap()));
    assert_eq!(service.get_endpoint().get_role(), Role::OnDemandFeed);
}

#[test]
fn a_service_without_a_provider_does_not_replay() {
    let endpoint = DXEndpoint::create().unwrap();
    let service = OnDemandService::get_instance_for(&endpoint).unwrap();
    assert_eq!(service.get_endpoint().get_role(), Role::Feed);
    assert!(!service.is_replay_supported().unwrap());
    assert!(!service.is_replay().unwrap());
    assert!(!service.is_clear().unwrap());
    assert_eq!(service.get_time().unwrap(), 0);
    assert_eq!(service.get_speed().unwrap(), 0.0);

    assert!(service.replay(1_273_175_268_000).is_err());
    assert!(service.replay_with_speed(1_273_175_268_000, 2.0).is_err());
    assert!(service.set_speed(1.0).is_err(), "no replay to speed up");
    service.set_speed(0.0).unwrap();
    service.pause().unwrap();
    assert!(!service.is_replay().unwrap());
}

#[test]
fn stop_and_clear_and_stop_and_resume_switch_the_clear_mode() {
    let endpoint = DXEndpoint::create().unwrap();
    let service = OnDemandService::get_instance_for(&endpoint).unwrap();
    service.stop_and_clear().unwrap();
    assert!(service.is_clear().unwrap());
    service.stop_and_resume().unwrap();
    assert!(!service.is_clear().unwrap());
}

#[test]
fn an_on_demand_address_supports_the_replay() {
    // Nothing listens there: the replay is supported, but no data comes.
    let endpoint = DXEndpoint::create_with_role(Role::OnDemandFeed).unwrap();
    endpoint.connect("(ondemand:127.0.0.1:1)").unwrap();
    let service = OnDemandService::get_instance_for(&endpoint).unwrap();
    assert!(service.is_replay_supported().unwrap());
    service.replay_with_speed(1_273_175_268_000, 2.0).unwrap();
    assert!(service.is_replay().unwrap());
    assert_eq!(service.get_speed().unwrap(), 2.0);
    service.pause().unwrap();
    assert_eq!(service.get_speed().unwrap(), 0.0);
    service.stop_and_resume().unwrap();
    assert!(!service.is_replay().unwrap());
    endpoint.close().unwrap();
}

#[test]
fn a_publisher_has_no_service() {
    let endpoint = DXEndpoint::create_with_role(Role::Publisher).unwrap();
    assert!(OnDemandService::get_instance_for(&endpoint).is_err());
}

/// As in Java, a change of the clear mode alone is not reported.
#[test]
fn a_change_of_the_clear_mode_alone_is_not_reported() {
    let endpoint = DXEndpoint::create().unwrap();
    let service = OnDemandService::get_instance_for(&endpoint).unwrap();
    let (sender, receiver) = mpsc::channel();
    service.add_property_change_listener(listener(sender)).unwrap();
    service.stop_and_clear().unwrap();
    assert!(service.is_clear().unwrap());
    assert!(receiver.recv_timeout(Duration::from_millis(500)).is_err());
}

/// The replay reports its mode and speed on a thread of the executor; a removed listener gets no more.
#[test]
fn a_listener_gets_the_changes_of_the_replay() {
    let endpoint = DXEndpoint::create_with_role(Role::OnDemandFeed).unwrap();
    endpoint.connect("(ondemand:127.0.0.1:1)").unwrap();
    let service = OnDemandService::get_instance_for(&endpoint).unwrap();
    let (sender, receiver) = mpsc::channel();
    let id = service.add_property_change_listener(listener(sender)).unwrap();

    service.replay_with_speed(1_273_175_268_000, 2.0).unwrap();
    let (_, old, new, thread) = change_of(&receiver, "replay");
    assert_eq!(
        (old, new),
        (PropertyValue::Boolean(false), PropertyValue::Boolean(true))
    );
    assert_ne!(thread, thread::current().id());
    // The properties of one change come in a row: replay, clear, speed, time.
    let (_, old, new, _) = change_of(&receiver, "speed");
    assert_eq!((old, new), (PropertyValue::Double(0.0), PropertyValue::Double(2.0)));
    service.set_speed(3.0).unwrap();
    let (_, old, new, _) = change_of(&receiver, "speed");
    assert_eq!((old, new), (PropertyValue::Double(2.0), PropertyValue::Double(3.0)));
    service.pause().unwrap();
    let (_, old, new, _) = change_of(&receiver, "speed");
    assert_eq!((old, new), (PropertyValue::Double(3.0), PropertyValue::Double(0.0)));

    service.stop_and_clear().unwrap();
    let (_, old, new, _) = change_of(&receiver, "replay");
    assert_eq!(
        (old, new),
        (PropertyValue::Boolean(true), PropertyValue::Boolean(false))
    );
    // Java may report the replay before it sets the clear mode, and then misses the change of the clear mode.
    assert!(service.is_clear().unwrap());

    service.remove_property_change_listener(id).unwrap();
    assert!(service.remove_property_change_listener(id).is_err());
    service.replay_with_speed(1_273_175_268_000, 1.0).unwrap();
    // The removed listener is dropped with its sender: past the changes it sent before, the channel disconnects.
    loop {
        match receiver.recv_timeout(Duration::from_secs(5)) {
            Ok(_) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => panic!("the removed listener was not dropped"),
        }
    }
    endpoint.close().unwrap();
}

/// A panic in a listener is caught and the listener stays.
#[test]
fn a_panicking_listener_stays() {
    let endpoint = DXEndpoint::create_with_role(Role::OnDemandFeed).unwrap();
    endpoint.connect("(ondemand:127.0.0.1:1)").unwrap();
    let service = OnDemandService::get_instance_for(&endpoint).unwrap();
    let (sender, receiver) = mpsc::channel();
    let mut first = true;
    service
        .add_property_change_listener(move |name, old, new| {
            if std::mem::take(&mut first) {
                panic!("the first change");
            }
            let _ = sender.send((name.map(str::to_owned), old, new, thread::current().id()));
        })
        .unwrap();
    // The replay changes several properties: the listener panics at the first and gets the others.
    service.replay_with_speed(1_273_175_268_000, 2.0).unwrap();
    receiver.recv_timeout(Duration::from_secs(5)).unwrap();
    // The speed is set after the panic, so its change reaches the listener.
    service.set_speed(3.0).unwrap();
    loop {
        let (_, _, new, _) = change_of(&receiver, "speed");
        if new == PropertyValue::Double(3.0) {
            break;
        }
    }
    endpoint.close().unwrap();
}
