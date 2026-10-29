//! Endpoints, subscriptions, listeners and publishing over `LOCAL_HUB` endpoints: no network needed.

mod support;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Barrier, Mutex};
use std::time::Duration;

use dxfeed_graal::event::{Quote, Trade};
use dxfeed_graal::{Endpoint, Error, Event, EventType, ManualExecutor, Role, Subscription};
use support::{Hub, collect, quote, quote_with_bid, trade};

/// The quote as `LOCAL_HUB` delivers it: the SDK does not carry the event time, the millisecond-sequence and the
/// nanosecond part of a quote through the hub, they come back as zero.
fn delivered_quote(sent: Quote) -> Quote {
    Quote {
        event_time: 0,
        time_millis_sequence: 0,
        time_nano_part: 0,
        ..sent
    }
}

/// The trade as `LOCAL_HUB` delivers it: the SDK does not carry the event time and the nanosecond part of a trade
/// through the hub, they come back as zero.
fn delivered_trade(sent: Trade) -> Trade {
    Trade {
        event_time: 0,
        time_nano_part: 0,
        ..sent
    }
}

fn received(receiver: &Receiver<Event>) -> Vec<Event> {
    receiver.try_iter().collect()
}

#[test]
fn local_hub_builds() {
    let hub = Hub::new();
    assert_eq!(hub.endpoint.role().unwrap(), Role::LocalHub);
    hub.executor.process_all_pending().unwrap();
}

#[test]
fn create_with_role() {
    assert_eq!(
        Endpoint::create(Role::Publisher).unwrap().role().unwrap(),
        Role::Publisher
    );
}

#[test]
fn close_then_drop() {
    let hub = Hub::new();
    hub.endpoint.close().unwrap();
    hub.endpoint.close().unwrap();
}

#[test]
fn endpoint_moves_to_another_thread() {
    let endpoint = Hub::new().endpoint;
    std::thread::spawn(move || {
        assert_eq!(endpoint.role().unwrap(), Role::LocalHub);
        drop(endpoint);
    })
    .join()
    .unwrap();
}

#[test]
fn types_are_send_and_sync() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<Endpoint>();
    send_sync::<dxfeed_graal::Feed>();
    send_sync::<dxfeed_graal::Publisher>();
    send_sync::<ManualExecutor>();
    send_sync::<Subscription>();
}

#[test]
fn symbols_and_listeners() {
    let hub = Hub::new();
    let subscription = hub
        .feed
        .create_subscription(&[EventType::Quote, EventType::Trade])
        .unwrap();
    // Listeners go before symbols: the SDK refuses a listener for a subscription that has symbols.
    let id = subscription.add_listener(|_| {}).unwrap();
    subscription.add_symbols(&["AAPL", "IBM"]).unwrap();
    subscription.remove_symbols(&["IBM"]).unwrap();
    assert!(matches!(
        subscription.add_symbols(&["A\0B"]),
        Err(Error::InvalidArgument(_))
    ));
    assert!(subscription.add_listener(|_| {}).is_err());
    subscription.remove_listener(id).unwrap();
    assert!(matches!(
        subscription.remove_listener(id),
        Err(Error::InvalidArgument(_))
    ));
    assert!(matches!(
        hub.feed.create_subscription(&[]),
        Err(Error::InvalidArgument(_))
    ));
    subscription.close().unwrap();
}

#[test]
fn quote_round_trip() {
    let hub = Hub::new();
    let subscription = hub.feed.create_subscription(&[EventType::Quote]).unwrap();
    let events = collect(&subscription);
    subscription.add_symbols(&["AAPL"]).unwrap();
    hub.publish_and_process(&[quote("AAPL").into()]);
    assert_eq!(received(&events), vec![Event::Quote(delivered_quote(quote("AAPL")))]);
}

#[test]
fn trade_round_trip() {
    let hub = Hub::new();
    let subscription = hub.feed.create_subscription(&[EventType::Trade]).unwrap();
    let events = collect(&subscription);
    subscription.add_symbols(&["AAPL"]).unwrap();
    hub.publish_and_process(&[trade("AAPL").into()]);
    assert_eq!(received(&events), vec![Event::Trade(delivered_trade(trade("AAPL")))]);
}

#[test]
fn invalid_events_are_rejected() {
    let hub = Hub::new();
    let wide = '\u{1F600}'; // Does not fit a Java char.
    let events: [Event; 3] = [
        Quote {
            bid_exchange_code: wide,
            ..quote("AAPL")
        }
        .into(),
        Trade {
            exchange_code: wide,
            ..trade("AAPL")
        }
        .into(),
        quote("A\0B").into(),
    ];
    for event in events {
        assert!(
            matches!(hub.publisher.publish(&[event]), Err(Error::InvalidArgument(_))),
            "an invalid event was accepted"
        );
    }
}

#[test]
fn quote_and_trade_in_order() {
    let hub = Hub::new();
    let subscription = hub
        .feed
        .create_subscription(&[EventType::Quote, EventType::Trade])
        .unwrap();
    let events = collect(&subscription);
    subscription.add_symbols(&["IBM"]).unwrap();
    hub.publish_and_process(&[quote("IBM").into(), trade("IBM").into()]);
    let got = received(&events);
    assert!(matches!(got.as_slice(), [Event::Quote(_), Event::Trade(_)]), "{got:?}");
}

#[test]
fn unsubscribed_symbol_is_not_delivered() {
    let hub = Hub::new();
    let subscription = hub.feed.create_subscription(&[EventType::Quote]).unwrap();
    let events = collect(&subscription);
    subscription.add_symbols(&["AAPL"]).unwrap();
    hub.publish_and_process(&[quote("MSFT").into()]);
    assert!(received(&events).is_empty());
}

#[test]
fn hubs_are_isolated() {
    // Run in parallel with the other tests that publish AAPL: each hub sees only its own events.
    for _ in 0..20 {
        let hub = Hub::new();
        let subscription = hub.feed.create_subscription(&[EventType::Quote]).unwrap();
        let events = collect(&subscription);
        subscription.add_symbols(&["AAPL"]).unwrap();
        let mine = quote_with_bid("AAPL", 1.0);
        hub.publish_and_process(&[mine.clone().into()]);
        assert_eq!(received(&events), vec![Event::Quote(delivered_quote(mine))]);
    }
}

#[test]
fn panic_in_listener_keeps_it_active() {
    let hub = Hub::new();
    let subscription = hub.feed.create_subscription(&[EventType::Quote]).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    subscription
        .add_listener(move |_| {
            if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                panic!("first batch");
            }
        })
        .unwrap();
    subscription.add_symbols(&["AAPL"]).unwrap();
    hub.publish_and_process(&[quote("AAPL").into()]);
    hub.publish_and_process(&[quote_with_bid("AAPL", 1.0).into()]);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn subscription_dropped_in_own_listener() {
    let hub = Hub::new();
    let slot: Arc<Mutex<Option<Subscription>>> = Arc::default();
    let calls = Arc::new(AtomicUsize::new(0));
    let subscription = hub.feed.create_subscription(&[EventType::Quote]).unwrap();
    let (s, c) = (slot.clone(), calls.clone());
    subscription
        .add_listener(move |_| {
            c.fetch_add(1, Ordering::SeqCst);
            let own = s.lock().unwrap().take();
            drop(own);
        })
        .unwrap();
    subscription.add_symbols(&["AAPL"]).unwrap();
    *slot.lock().unwrap() = Some(subscription);
    hub.publish_and_process(&[quote("AAPL").into()]);
    hub.publish_and_process(&[quote_with_bid("AAPL", 1.0).into()]);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(slot.lock().unwrap().is_none());
}

#[test]
fn remove_listener_waits_for_running_call() {
    let hub = Arc::new(Hub::new());
    let subscription = hub.feed.create_subscription(&[EventType::Quote]).unwrap();
    let started = Arc::new(Barrier::new(2));
    let finished = Arc::new(AtomicBool::new(false));
    let (b, f) = (started.clone(), finished.clone());
    let id = subscription
        .add_listener(move |_| {
            b.wait();
            std::thread::sleep(Duration::from_millis(300));
            f.store(true, Ordering::SeqCst);
        })
        .unwrap();
    subscription.add_symbols(&["AAPL"]).unwrap();
    hub.publisher.publish(&[quote("AAPL").into()]).unwrap();
    let processor = {
        let hub = hub.clone();
        std::thread::spawn(move || hub.executor.process_all_pending().unwrap())
    };
    started.wait();
    subscription.remove_listener(id).unwrap();
    assert!(
        finished.load(Ordering::SeqCst),
        "remove_listener returned while the call was running"
    );
    processor.join().unwrap();
}

#[test]
fn concurrent_processing_delivers_each_event_once() {
    let hub = Arc::new(Hub::new());
    let subscription = hub.feed.create_subscription(&[EventType::Quote]).unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let c = count.clone();
    subscription
        .add_listener(move |events| _ = c.fetch_add(events.len(), Ordering::SeqCst))
        .unwrap();
    let symbols: Vec<String> = (0..200).map(|i| format!("S{i}")).collect();
    subscription.add_symbols(&symbols).unwrap();
    let events: Vec<Event> = symbols.iter().map(|s| quote(s).into()).collect();
    for chunk in events.chunks(10) {
        hub.publisher.publish(chunk).unwrap();
    }
    let threads: Vec<_> = (0..2)
        .map(|_| {
            let hub = hub.clone();
            std::thread::spawn(move || hub.executor.process_all_pending().unwrap())
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    hub.executor.process_all_pending().unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 200);
}

#[test]
fn processing_from_listener_does_not_nest_calls() {
    let hub = Arc::new(Hub::new());
    let subscription = hub.feed.create_subscription(&[EventType::Quote]).unwrap();
    let (calls, inside, nested) = (
        Arc::new(AtomicUsize::new(0)),
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    );
    let (h, c, i, n) = (Arc::downgrade(&hub), calls.clone(), inside.clone(), nested.clone());
    subscription
        .add_listener(move |_| {
            if i.swap(true, Ordering::SeqCst) {
                n.store(true, Ordering::SeqCst);
            }
            if c.fetch_add(1, Ordering::SeqCst) == 0
                && let Some(hub) = h.upgrade()
            {
                // The SDK serializes the calls of a subscription: this quote comes in a second call after this one.
                hub.publish_and_process(&[quote_with_bid("AAPL", 1.0).into()]);
            }
            i.store(false, Ordering::SeqCst);
        })
        .unwrap();
    subscription.add_symbols(&["AAPL"]).unwrap();
    hub.publish_and_process(&[quote("AAPL").into()]);
    hub.executor.process_all_pending().unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(!nested.load(Ordering::SeqCst));
}

#[test]
fn listeners_drop_each_other_without_deadlock() {
    let hub = Hub::with_default_executor();
    let slots: [Arc<Mutex<Option<Subscription>>>; 2] = [Arc::default(), Arc::default()];
    let started: Arc<[AtomicBool; 2]> = Arc::default();
    let (done, wait) = mpsc::channel();
    for i in 0..2 {
        let subscription = hub.feed.create_subscription(&[EventType::Quote]).unwrap();
        let (other, started, done) = (slots[1 - i].clone(), started.clone(), done.clone());
        subscription
            .add_listener(move |_| {
                // Wait a little for the other listener, so both drop each other while both run, if the executor
                // runs them on two threads.
                started[i].store(true, Ordering::SeqCst);
                let until = std::time::Instant::now() + Duration::from_secs(1);
                while !started[1 - i].load(Ordering::SeqCst) && std::time::Instant::now() < until {
                    std::thread::yield_now();
                }
                let taken = other.lock().unwrap().take();
                drop(taken);
                let _ = done.send(i);
            })
            .unwrap();
        subscription.add_symbols(&["AAPL"]).unwrap();
        *slots[i].lock().unwrap() = Some(subscription);
    }
    hub.publisher.publish(&[quote("AAPL").into()]).unwrap();
    wait.recv_timeout(Duration::from_secs(10)).expect("no listener ran");
    let _ = wait.recv_timeout(Duration::from_secs(2));
    // Whatever the listeners left is dropped here; neither must hang.
    let (cleaned, cleanup) = mpsc::channel();
    let remaining = slots.clone();
    std::thread::spawn(move || {
        for slot in remaining {
            let taken = slot.lock().unwrap().take();
            drop(taken);
        }
        cleaned.send(()).unwrap();
    });
    cleanup.recv_timeout(Duration::from_secs(10)).expect("deadlock");
    while wait.recv_timeout(Duration::from_millis(500)).is_ok() {}
    hub.publisher.publish(&[quote_with_bid("AAPL", 2.0).into()]).unwrap();
    assert!(
        wait.recv_timeout(Duration::from_millis(500)).is_err(),
        "a dropped subscription still receives"
    );
}

#[test]
fn hub_with_pending_tasks_drops_quickly() {
    let hub = Hub::new();
    let subscription = hub.feed.create_subscription(&[EventType::Quote]).unwrap();
    subscription.add_listener(|_| {}).unwrap();
    subscription.add_symbols(&["AAPL"]).unwrap();
    hub.publisher.publish(&[quote("AAPL").into()]).unwrap();
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        drop(subscription);
        drop(hub);
        sender.send(()).unwrap();
    });
    receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("dropping the hub took over 5 seconds");
}
