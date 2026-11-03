//! Event queues on local hubs: the polls, the wake-ups, the closing, the tags of a shared queue and the work of its
//! executor; no network needed.

mod support;

use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use dxfeed_graal::event::{Candle, Quote, TimeAndSale, Trade};
use dxfeed_graal::event_queue::{EventQueue, PollResult, SharedEventQueue};
use dxfeed_graal::ipf::InstrumentProfile;
use dxfeed_graal::ipf::live::InstrumentProfileCollector;
use dxfeed_graal::model::TimeSeriesEventModel;
use dxfeed_graal::{Error, Event, Executor, Promise, system};
use support::{Hub, candle, is_child, quote, run_child, time_and_sale, trade};

const WAIT: Duration = Duration::from_secs(5);

/// Polls `queue` until it returns `count` events or nothing for [`WAIT`].
fn poll_events(queue: &SharedEventQueue, count: usize) -> Vec<(i64, Event)> {
    let mut events = Vec::new();
    while events.len() < count {
        match queue.poll(Some(WAIT), 0).unwrap() {
            PollResult::Events(more) => events.extend(more),
            other => panic!("{other:?} after {events:?}"),
        }
    }
    events
}

/// Polls `queue` until `received` gets a value; the work of the executor of the queue runs in the polls.
fn poll_until<T>(queue: &SharedEventQueue, received: &mpsc::Receiver<T>) -> T {
    for _ in 0..50 {
        if let Ok(value) = received.try_recv() {
            return value;
        }
        assert_ne!(
            queue.poll(Some(Duration::from_millis(100)), 0).unwrap(),
            PollResult::Closed
        );
    }
    panic!("nothing ran in the polls");
}

fn is_illegal_state(result: Result<impl std::fmt::Debug, Error>) -> bool {
    matches!(result, Err(Error::Java(e)) if e.get_class_name() == "java.lang.IllegalStateException")
}

#[test]
fn a_queue_polls_the_events_of_its_subscription_on_this_thread() {
    let hub = Hub::with_default_executor();
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    let queue = EventQueue::new(&subscription).unwrap();
    let (sender, listened) = mpsc::channel();
    subscription
        .add_event_listener(move |events| {
            let _ = sender.send((events.count(), thread::current().id()));
        })
        .unwrap();
    subscription.add_symbols(["AAPL"]).unwrap();
    assert_eq!(queue.poll(Some(Duration::ZERO), 0).unwrap(), PollResult::NoEvents);

    hub.publisher.publish_events(&[quote("AAPL").into()]).unwrap();
    let PollResult::Events(quotes) = queue.poll(Some(WAIT), 0).unwrap() else {
        panic!("no quote");
    };
    assert_eq!(quotes.len(), 1);
    assert_eq!(quotes[0].symbol, "AAPL");
    assert_eq!(listened.try_recv().unwrap(), (1, thread::current().id()));
    assert_eq!(queue.poll(Some(Duration::ZERO), 0).unwrap(), PollResult::NoEvents);
    assert_eq!(queue.get_dropped_events().unwrap(), 0);
    assert!(format!("{queue:?}").starts_with("EventQueue"));
}

#[test]
fn a_wake_up_ends_a_poll_without_events() {
    let hub = Hub::with_default_executor();
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    let queue = EventQueue::new(&subscription).unwrap();
    thread::scope(|scope| {
        scope.spawn(|| {
            thread::sleep(Duration::from_millis(200));
            queue.wake_up().unwrap();
        });
        let start = Instant::now();
        assert_eq!(queue.poll(Some(WAIT), 0).unwrap(), PollResult::NoEvents);
        assert!(start.elapsed() < WAIT / 2, "{:?}", start.elapsed());
    });
    // Several wake-ups before a poll wake up one poll: the next one waits for its timeout.
    queue.wake_up().unwrap();
    queue.wake_up().unwrap();
    let start = Instant::now();
    assert_eq!(queue.poll(Some(WAIT), 0).unwrap(), PollResult::NoEvents);
    assert!(start.elapsed() < WAIT / 2, "{:?}", start.elapsed());
    let start = Instant::now();
    assert_eq!(
        queue.poll(Some(Duration::from_millis(300)), 0).unwrap(),
        PollResult::NoEvents
    );
    assert!(start.elapsed() >= Duration::from_millis(250), "{:?}", start.elapsed());
}

#[test]
fn closing_ends_a_poll_and_the_later_ones() {
    let hub = Hub::with_default_executor();
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    let queue = EventQueue::new(&subscription).unwrap();
    thread::scope(|scope| {
        scope.spawn(|| {
            thread::sleep(Duration::from_millis(200));
            queue.close().unwrap();
        });
        let start = Instant::now();
        assert_eq!(queue.poll(Some(WAIT), 0).unwrap(), PollResult::Closed);
        assert!(start.elapsed() < WAIT / 2, "{:?}", start.elapsed());
    });
    assert_eq!(queue.poll(Some(Duration::ZERO), 0).unwrap(), PollResult::Closed);
    queue.close().unwrap();
}

#[test]
fn closing_the_subscription_closes_its_queue() {
    let hub = Hub::with_default_executor();
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    let queue = EventQueue::new(&subscription).unwrap();
    subscription.close().unwrap();
    assert_eq!(queue.poll(Some(WAIT), 0).unwrap(), PollResult::Closed);
}

#[test]
fn a_shared_queue_tags_the_events_of_its_subscriptions() {
    let hub = Hub::with_default_executor();
    let quotes = hub.feed.create_subscription::<Quote>().unwrap();
    let trades = hub.feed.create_subscription::<Trade>().unwrap();
    let queue = SharedEventQueue::new().unwrap();
    queue.attach(&quotes, 1).unwrap();
    queue.attach(&trades, 2).unwrap();
    quotes.add_symbols(["AAPL"]).unwrap();
    trades.add_symbols(["IBM"]).unwrap();
    hub.publisher
        .publish_events(&[quote("AAPL").into(), trade("IBM").into()])
        .unwrap();

    let mut events = poll_events(&queue, 2);
    events.sort_by_key(|&(tag, _)| tag);
    assert!(
        matches!(&events[0], (1, Event::Quote(q)) if q.symbol == "AAPL"),
        "{events:?}"
    );
    assert!(
        matches!(&events[1], (2, Event::Trade(t)) if t.symbol == "IBM"),
        "{events:?}"
    );
    assert_eq!(queue.get_dropped_events().unwrap(), 0);

    // A subscription leaves by being closed; the queue stays open. A trade that changes nothing would not come again.
    quotes.close().unwrap();
    let changed = Trade {
        price: 191.0,
        ..trade("IBM")
    };
    hub.publisher.publish_events(&[changed.into()]).unwrap();
    assert!(matches!(&poll_events(&queue, 1)[..], [(2, Event::Trade(_))]));
    assert!(format!("{queue:?}").starts_with("SharedEventQueue"));
}

#[test]
fn a_subscription_with_an_executor_or_a_queue_is_refused() {
    let hub = Hub::with_default_executor();
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    subscription.set_executor(Some(&hub.executor)).unwrap();
    assert!(is_illegal_state(EventQueue::new(&subscription)));
    assert!(is_illegal_state(
        SharedEventQueue::new().unwrap().attach(&subscription, 0)
    ));

    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    let _queue = EventQueue::new(&subscription).unwrap();
    assert!(is_illegal_state(EventQueue::new(&subscription)));
}

#[test]
fn the_executor_of_a_shared_queue_runs_listeners_in_the_polls() {
    let queue = SharedEventQueue::new().unwrap();
    let executor = queue.get_executor().unwrap();
    let collector = InstrumentProfileCollector::new().unwrap();
    collector.set_executor(&executor).unwrap();
    // The queue stays open while an executor it lent lives, and the collector keeps its executor.
    drop(executor);
    let (sender, updates) = mpsc::channel();
    collector
        .add_update_listener(move |profiles| {
            let _ = sender.send((profiles.len(), thread::current().id()));
        })
        .unwrap();
    let profile = InstrumentProfile {
        instrument_type: "STOCK".into(),
        symbol: "AAPL".into(),
        ..InstrumentProfile::new()
    };
    collector.update_instrument_profile(&profile).unwrap();
    assert!(
        updates.recv_timeout(Duration::from_millis(300)).is_err(),
        "no poll ran yet"
    );
    let (count, thread) = poll_until(&queue, &updates);
    assert!(count >= 1);
    assert_eq!(thread, thread::current().id());
}

#[test]
fn a_model_on_the_executor_of_a_shared_queue_notifies_in_the_polls() {
    let hub = Hub::with_default_executor();
    let queue = SharedEventQueue::new().unwrap();
    let model = TimeSeriesEventModel::<Candle>::new().unwrap();
    let executor = queue.get_executor().unwrap();
    model.set_executor(Some(&executor)).unwrap();
    drop(executor);
    model.set_symbol(Some("C{=d}")).unwrap();
    model.set_from_time(1_000).unwrap();
    let candles = model.get_events_list().unwrap();
    let (sender, changes) = mpsc::channel();
    candles
        .add_listener(move |candles| {
            let _ = sender.send((candles.count(), thread::current().id()));
        })
        .unwrap();
    model.attach(&hub.feed).unwrap();
    let mut published = candle("C{=d}");
    published.set_time(2_000);
    hub.publisher.publish_events(&[published.into()]).unwrap();
    assert_eq!(poll_until(&queue, &changes), (1, thread::current().id()));
    model.close().unwrap();
}

#[test]
fn a_promise_handler_on_a_shared_queue_runs_in_a_poll() {
    let queue = SharedEventQueue::new().unwrap();
    let promise = Promise::<()>::new().unwrap();
    let (sender, done) = mpsc::channel();
    promise
        .when_done_async(
            move |promise| {
                let _ = sender.send((promise.is_done().unwrap(), thread::current().id()));
            },
            &queue.get_executor().unwrap(),
        )
        .unwrap();
    thread::scope(|scope| {
        scope.spawn(|| promise.complete().unwrap()).join().unwrap();
    });
    assert!(done.try_recv().is_err(), "no poll ran yet");
    assert_eq!(poll_until(&queue, &done), (true, thread::current().id()));
}

#[test]
fn the_executor_of_a_closed_queue_runs_the_work_at_once() {
    let queue = SharedEventQueue::new().unwrap();
    let executor: Executor = queue.get_executor().unwrap();
    queue.close().unwrap();
    let promise = Promise::<()>::new().unwrap();
    let (sender, done) = mpsc::channel();
    promise
        .when_done_async(move |_| sender.send(thread::current().id()).unwrap(), &executor)
        .unwrap();
    promise.complete().unwrap();
    // The work runs on the posting thread, but the listeners of the application are not called.
    assert!(done.recv_timeout(Duration::from_millis(300)).is_err());
    assert_eq!(queue.poll(Some(Duration::ZERO), 0).unwrap(), PollResult::Closed);
}

#[test]
fn a_poll_inside_a_listener_is_refused() {
    let hub = Hub::new();
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    let queue = Arc::new(SharedEventQueue::new().unwrap());
    let (sender, polled) = mpsc::channel();
    let inner = Arc::clone(&queue);
    subscription
        .add_event_listener(move |_| {
            let _ = sender.send(matches!(inner.poll(Some(Duration::ZERO), 0), Err(Error::InCallback)));
        })
        .unwrap();
    subscription.add_symbols(["AAPL"]).unwrap();
    hub.publish_and_process(&[quote("AAPL").into()]);
    assert!(polled.recv_timeout(WAIT).unwrap());
}

#[test]
fn a_closed_queue_lets_the_endpoint_terminate() {
    let hub = Hub::with_default_executor();
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    let queue = EventQueue::new(&subscription).unwrap();
    subscription.add_symbols(["AAPL"]).unwrap();
    hub.publisher.publish_events(&[quote("AAPL").into()]).unwrap();
    // Let the hub deliver the quote to the subscription, so that the queue holds it.
    thread::sleep(Duration::from_millis(300));
    queue.close().unwrap();
    assert_eq!(
        queue.get_dropped_events().unwrap(),
        1,
        "the quote that no poll returned"
    );
    let (sender, terminated) = mpsc::channel();
    let endpoint = hub.endpoint;
    thread::spawn(move || sender.send(endpoint.close_and_await_termination().is_ok()));
    assert!(terminated.recv_timeout(Duration::from_secs(10)).unwrap());
}

#[test]
fn the_endpoint_awaits_the_events_that_the_queue_holds() {
    let hub = Hub::with_default_executor();
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    let queue = EventQueue::new(&subscription).unwrap();
    subscription.add_symbols(["AAPL"]).unwrap();
    hub.publisher.publish_events(&[quote("AAPL").into()]).unwrap();
    thread::sleep(Duration::from_millis(300));
    let (sender, terminated) = mpsc::channel();
    let endpoint = hub.endpoint;
    thread::spawn(move || sender.send(endpoint.close_and_await_termination().is_ok()));
    assert!(
        terminated.recv_timeout(Duration::from_millis(500)).is_err(),
        "it waits for a poll"
    );
    assert!(matches!(queue.poll(Some(WAIT), 0).unwrap(), PollResult::Events(quotes) if quotes.len() == 1));
    assert!(terminated.recv_timeout(Duration::from_secs(10)).unwrap());
}

#[test]
fn a_full_buffer_drops_the_oldest_events() {
    run_child("child_a_full_buffer_drops_the_oldest_events");
}

#[test]
#[ignore = "run by a_full_buffer_drops_the_oldest_events"]
fn child_a_full_buffer_drops_the_oldest_events() {
    if !is_child() {
        return;
    }
    system::set_property("com.devexperts.qd.impl.matrix.Agent.MaxBufferSize", "10").unwrap();
    let hub = Hub::with_default_executor();
    let subscription = hub.feed.create_subscription::<TimeAndSale>().unwrap();
    let queue = EventQueue::new(&subscription).unwrap();
    subscription.add_symbols(["AAPL"]).unwrap();
    let events: Vec<Event> = (0..100).map(|_| time_and_sale("AAPL").into()).collect();
    hub.publisher.publish_events(&events).unwrap();
    // Let the hub deliver the events into the buffer of the subscription before the poll takes them.
    thread::sleep(Duration::from_millis(500));
    let PollResult::Events(sales) = queue.poll(Some(WAIT), 0).unwrap() else {
        panic!("no events");
    };
    let dropped = queue.get_dropped_events().unwrap();
    assert!(dropped > 0, "{} events, none dropped", sales.len());
    assert_eq!(subscription.get_total_dropped_events().unwrap(), dropped);
}
