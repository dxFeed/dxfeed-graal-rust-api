//! Event streams over the event queue of their subscription ([`StreamOptions::event_queue`]), on `LOCAL_HUB` endpoints:
//! the SDK behavior they rest on, the conflation on demand, the wakes and the end of a stream.

mod support;

use std::sync::{Arc, Mutex, MutexGuard, PoisonError, mpsc};
use std::task::{Context, Poll, Wake, Waker};
use std::thread;
use std::time::{Duration, Instant};

use dxfeed_graal::event::{Quote, SubscriptionItem, TimeAndSale, Trade};
use dxfeed_graal::event_queue::{EventQueue, PollResult};
use dxfeed_graal::{
    DXFeedSubscription, Error, Event, EventStream, EventType, JavaExceptionClass, StreamOptions, live_events,
    live_handles,
};
use support::{CountingWaker, Hub, block_on_with, quote_with_bid, time_and_sale, trade};

/// Runs the tests of this file one by one, so that each can count the live handles.
fn serial() -> MutexGuard<'static, ()> {
    static SERIAL: Mutex<()> = Mutex::new(());
    SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
}

const WAIT: Duration = Duration::from_secs(5);
const LONG: Duration = Duration::from_secs(10);

/// Lets the hub deliver what was published to the subscriptions, so that their queues hold it.
fn settle() {
    thread::sleep(Duration::from_millis(300));
}

/// Waits until `done` holds, for at most `LONG`; returns whether it held.
fn eventually(mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + LONG;
    while !done() {
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(10));
    }
    true
}

/// Polls `stream` once with `waker`.
fn poll<E, S>(stream: &mut EventStream<E, S>, waker: &Arc<CountingWaker>) -> Poll<Option<E>>
where
    E: SubscriptionItem,
    S: AsRef<DXFeedSubscription<E>>,
{
    let waker = Waker::from(Arc::clone(waker));
    stream.poll_next(&mut Context::from_waker(&waker))
}

/// The next item of `stream`, waiting for it on this thread; `None` if nothing comes within `LONG`.
fn next<E, S>(stream: &mut EventStream<E, S>) -> Option<Option<E>>
where
    E: SubscriptionItem,
    S: AsRef<DXFeedSubscription<E>>,
{
    block_on_with(stream.next(), &CountingWaker::new(), LONG)
}

/// The quote that `stream` yields next, waiting for it.
fn next_quote<S: AsRef<DXFeedSubscription<Quote>>>(stream: &mut EventStream<Quote, S>) -> Quote {
    match next(stream) {
        Some(Some(quote)) => quote,
        other => panic!("no quote: {other:?}"),
    }
}

/// Publishes a quote of `symbol` with `bid` for each of `bids`, letting the hub deliver each.
fn publish_bids(hub: &Hub, symbol: &str, bids: &[f64]) {
    for &bid in bids {
        hub.publisher
            .publish_events(&[quote_with_bid(symbol, bid).into()])
            .unwrap();
        settle();
    }
}

/// A queue stream of the quotes of S0..S4.
fn quotes(hub: &Hub) -> EventStream<Quote> {
    let stream = hub
        .feed
        .create_subscription::<Quote>()
        .unwrap()
        .into_stream_with(StreamOptions::event_queue())
        .unwrap();
    stream
        .subscription()
        .add_symbols(["S0", "S1", "S2", "S3", "S4"])
        .unwrap();
    stream
}

fn is_illegal_state<T: std::fmt::Debug>(result: Result<T, Error>) -> bool {
    matches!(&result, Err(Error::Java(e)) if e.get_class_name() == JavaExceptionClass::IllegalStateException.get_name())
}

/// Polls `queue` until it returns `count` events or a poll waits `WAIT` in vain.
fn poll_events<E: SubscriptionItem + TryFrom<Event> + std::fmt::Debug>(queue: &EventQueue<E>, count: usize) -> Vec<E> {
    let mut events = Vec::new();
    while events.len() < count {
        match queue.poll(Some(WAIT), 0).unwrap() {
            PollResult::Events(batch) => events.extend(batch),
            other => panic!("{other:?} after {} of {count} events", events.len()),
        }
    }
    events
}

// The SDK behavior that the streams rest on.

#[test]
fn an_unpolled_queue_keeps_the_last_quote_of_a_symbol() {
    let _serial = serial();
    let hub = Hub::with_default_executor();
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    let queue = EventQueue::new(&subscription).unwrap();
    subscription.add_symbols(["S0", "S1"]).unwrap();
    for bid in [1.0, 2.0, 3.0] {
        hub.publisher
            .publish_events(&[quote_with_bid("S0", bid).into()])
            .unwrap();
        hub.publisher
            .publish_events(&[quote_with_bid("S1", bid + 10.0).into()])
            .unwrap();
        settle();
    }
    let mut quotes = poll_events(&queue, 2);
    quotes.sort_by(|a, b| a.symbol.cmp(&b.symbol));
    let bids: Vec<_> = quotes.iter().map(|q| (q.symbol.as_str(), q.bid_price)).collect();
    assert_eq!(bids, [("S0", 3.0), ("S1", 13.0)]);
    assert_eq!(queue.poll(Some(Duration::ZERO), 0).unwrap(), PollResult::NoEvents);
    assert_eq!(queue.get_dropped_events().unwrap(), 0);
}

#[test]
fn an_unpolled_queue_keeps_every_time_and_sale() {
    let _serial = serial();
    let hub = Hub::with_default_executor();
    let subscription = hub.feed.create_subscription::<TimeAndSale>().unwrap();
    let queue = EventQueue::new(&subscription).unwrap();
    subscription.add_symbols(["S0"]).unwrap();
    for price in [1.0, 2.0, 3.0] {
        let sale = TimeAndSale {
            price,
            ..time_and_sale("S0")
        };
        hub.publisher.publish_events(&[sale.into()]).unwrap();
        settle();
    }
    let prices: Vec<_> = poll_events(&queue, 3).iter().map(|s| s.price).collect();
    assert_eq!(prices, [1.0, 2.0, 3.0]);
}

#[test]
fn a_poll_after_the_endpoint_closes_waits_for_a_wake_up() {
    let _serial = serial();
    let hub = Hub::with_default_executor();
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    let queue = EventQueue::new(&subscription).unwrap();
    subscription.add_symbols(["S0"]).unwrap();
    hub.endpoint.close().unwrap();
    let (sender, polled) = mpsc::channel();
    thread::scope(|scope| {
        scope.spawn(|| {
            let start = Instant::now();
            let result = queue.poll(Some(WAIT), 0).unwrap();
            sender.send((result, start.elapsed())).unwrap();
        });
        assert!(
            polled.recv_timeout(Duration::from_millis(500)).is_err(),
            "the poll ends before the wake-up"
        );
        queue.wake_up().unwrap();
        let (result, elapsed) = polled.recv_timeout(WAIT).unwrap();
        assert_eq!(result, PollResult::NoEvents);
        assert!(elapsed < WAIT, "{elapsed:?}");
    });
}

#[test]
fn a_drained_queue_lets_the_endpoint_terminate() {
    let _serial = serial();
    let hub = Hub::with_default_executor();
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    let queue = EventQueue::new(&subscription).unwrap();
    subscription.add_symbols(["S0"]).unwrap();
    publish_bids(&hub, "S0", &[1.0]);
    assert_eq!(poll_events(&queue, 1).len(), 1);
    let (sender, terminated) = mpsc::channel();
    let endpoint = hub.endpoint;
    thread::spawn(move || sender.send(endpoint.close_and_await_termination().is_ok()));
    assert!(terminated.recv_timeout(LONG).unwrap());
    assert_eq!(queue.poll(Some(Duration::ZERO), 0).unwrap(), PollResult::NoEvents);
}

// The delivery of the events.

/// The quotes of a symbol published while no task waits come as the last one.
#[test]
fn stream_conflates_the_quotes_published_while_no_task_waits() {
    let _serial = serial();
    let hub = Hub::with_default_executor();
    let mut stream = quotes(&hub);
    for bid in [1.0, 2.0, 3.0] {
        hub.publisher
            .publish_events(&[
                quote_with_bid("S0", bid).into(),
                quote_with_bid("S1", bid + 10.0).into(),
            ])
            .unwrap();
        settle();
    }
    let mut bids: Vec<_> = (0..2)
        .map(|_| next_quote(&mut stream))
        .map(|q| (q.symbol, q.bid_price))
        .collect();
    bids.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(bids, [("S0".to_owned(), 3.0), ("S1".to_owned(), 13.0)]);
    assert_eq!(stream.buffered(), 0);
    assert!(poll(&mut stream, &CountingWaker::new()).is_pending());
    assert_eq!((stream.dropped(), stream.skipped()), (0, 0));
}

/// While the buffer holds events and no task waits, the SDK keeps conflating the new ones.
#[test]
fn stream_takes_nothing_while_its_buffer_holds_events() {
    let _serial = serial();
    let hub = Hub::with_default_executor();
    let mut stream = quotes(&hub);
    let waker = CountingWaker::new();
    hub.publisher
        .publish_events(&[
            quote_with_bid("S0", 1.0).into(),
            quote_with_bid("S1", 1.0).into(),
            quote_with_bid("S2", 1.0).into(),
        ])
        .unwrap();
    settle();
    // The first request brings a batch (all three quotes unless the hub is slow), with one wake.
    assert!(poll(&mut stream, &waker).is_pending());
    assert!(eventually(|| stream.buffered() > 0), "{stream:?}");
    let buffered = stream.buffered();
    assert_eq!(waker.total(), 1);
    publish_bids(&hub, "S0", &[2.0, 3.0, 4.0]);
    assert_eq!(stream.buffered(), buffered, "taken from the SDK with no request");
    assert_eq!(waker.total(), 1, "woken with no task waiting");
    let first: Vec<_> = (0..buffered)
        .map(|_| match poll(&mut stream, &waker) {
            Poll::Ready(Some(q)) => q.symbol,
            other => panic!("not a buffered quote: {other:?}"),
        })
        .collect();
    // The rest: the symbols that the first batch missed, with S0 conflated to its last quote.
    let expected = 3 - buffered + usize::from(first.iter().any(|symbol| symbol == "S0"));
    let rest: Vec<_> = (0..expected)
        .map(|_| next_quote(&mut stream))
        .map(|q| (q.symbol, q.bid_price))
        .collect();
    let s0: Vec<_> = rest.iter().filter(|(symbol, _)| symbol == "S0").collect();
    assert_eq!(s0, [&("S0".to_owned(), 4.0)], "{rest:?}");
    assert!(poll(&mut stream, &waker).is_pending());
}

/// A time-series stream yields every time and sale.
#[test]
fn time_series_stream_keeps_every_event() {
    let _serial = serial();
    let hub = Hub::with_default_executor();
    let mut stream = hub
        .feed
        .create_time_series_subscription::<TimeAndSale>()
        .unwrap()
        .into_stream_with(StreamOptions::event_queue())
        .unwrap();
    stream.subscription().set_from_time(0).unwrap();
    stream.subscription().add_symbols(["S0"]).unwrap();
    for (n, price) in [1.0, 2.0, 3.0].into_iter().enumerate() {
        // A sale of a time series with the index of an earlier one would replace it.
        let sale = time_and_sale("S0");
        let sale = TimeAndSale {
            price,
            index: sale.index + n as i64,
            ..sale
        };
        hub.publisher.publish_events(&[sale.into()]).unwrap();
        settle();
    }
    // The SDK may give the sales of a time series newest first.
    let mut prices: Vec<_> = (0..3)
        .map(|_| match next(&mut stream) {
            Some(Some(sale)) => sale.price,
            other => panic!("no time and sale: {other:?}"),
        })
        .collect();
    prices.sort_by(f64::total_cmp);
    assert_eq!(prices, [1.0, 2.0, 3.0]);
}

/// An untyped stream yields the events of each of its types as [`Event`].
#[test]
fn untyped_stream_yields_events() {
    let _serial = serial();
    let hub = Hub::with_default_executor();
    let mut stream = hub
        .feed
        .create_subscription_with_types(&[EventType::Quote, EventType::Trade])
        .unwrap()
        .into_stream_with(StreamOptions::event_queue())
        .unwrap();
    stream.subscription().add_symbols(["S0"]).unwrap();
    hub.publisher
        .publish_events(&[quote_with_bid("S0", 1.0).into(), trade("S0").into()])
        .unwrap();
    let mut types: Vec<_> = (0..2)
        .map(|_| match next(&mut stream) {
            Some(Some(event)) => event.event_type(),
            other => panic!("no event: {other:?}"),
        })
        .collect();
    types.sort_by_key(|t| format!("{t:?}"));
    assert_eq!(types, [EventType::Quote, EventType::Trade]);
    assert_eq!(stream.skipped(), 0);
}

#[test]
fn options() {
    let options = StreamOptions::event_queue();
    assert_eq!(options.capacity(), None);
    assert_ne!(options, StreamOptions::unbounded());
    let _serial = serial();
    let hub = Hub::with_default_executor();
    let stream = quotes(&hub);
    assert_eq!((stream.buffered(), stream.dropped(), stream.skipped()), (0, 0, 0));
    assert!(format!("{stream:?}").contains("event_queue: true"), "{stream:?}");
}

/// The SDK refuses the queue of a subscription that has symbols on a feed, an executor or another queue.
#[test]
fn stream_needs_a_subscription_that_a_queue_can_serve() {
    let _serial = serial();
    let hub = Hub::with_default_executor();
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    subscription.add_symbols(["S0"]).unwrap();
    assert!(is_illegal_state(
        subscription.into_stream_with(StreamOptions::event_queue())
    ));
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    subscription.set_executor(Some(&hub.executor)).unwrap();
    assert!(is_illegal_state(
        subscription.into_stream_with(StreamOptions::event_queue())
    ));
    let subscription = hub.feed.create_subscription::<Trade>().unwrap();
    let _queue = EventQueue::new(&subscription).unwrap();
    assert!(is_illegal_state(
        subscription.into_stream_with(StreamOptions::event_queue())
    ));
}

/// A waker that formats the stream, which locks its buffer: it hangs if the stream wakes under that lock.
struct Reentrant(Arc<Mutex<Option<EventStream<Quote>>>>);

impl Wake for Reentrant {
    fn wake(self: Arc<Self>) {
        let stream = self.0.lock().unwrap();
        let _ = format!("{stream:?}");
    }
}

#[test]
fn wakes_outside_the_lock() {
    let _serial = serial();
    let hub = Hub::with_default_executor();
    let stream = Arc::new(Mutex::new(Some(quotes(&hub))));
    let waker = Waker::from(Arc::new(Reentrant(stream.clone())));
    let poll_again = || {
        stream
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .poll_next(&mut Context::from_waker(&waker))
    };
    assert!(poll_again().is_pending());
    publish_bids(&hub, "S0", &[1.0]);
    assert!(eventually(|| stream.lock().unwrap().as_ref().unwrap().buffered() == 1));
    assert!(matches!(poll_again(), Poll::Ready(Some(_))));

    // The end of the stream wakes outside the lock as well.
    assert!(poll_again().is_pending());
    hub.endpoint.close().unwrap();
    assert!(eventually(|| matches!(poll_again(), Poll::Ready(None))));
}

#[cfg(feature = "stream")]
#[test]
fn stream_trait() {
    use std::pin::Pin;

    use futures_core::Stream;

    let _serial = serial();
    let hub = Hub::with_default_executor();
    let mut stream = quotes(&hub);
    assert_eq!(Stream::size_hint(&stream), (0, None));
    let waker = Waker::from(CountingWaker::new());
    let mut cx = Context::from_waker(&waker);
    assert!(Stream::poll_next(Pin::new(&mut stream), &mut cx).is_pending());
    publish_bids(&hub, "S0", &[1.0]);
    assert!(eventually(|| Stream::size_hint(&stream) == (1, None)));
    stream.close().unwrap();
    assert_eq!(Stream::size_hint(&stream), (1, Some(1)));
    assert!(matches!(
        Stream::poll_next(Pin::new(&mut stream), &mut cx),
        Poll::Ready(Some(_))
    ));
    assert!(matches!(
        Stream::poll_next(Pin::new(&mut stream), &mut cx),
        Poll::Ready(None)
    ));
}

// The end of a stream.

/// A closed stream yields its buffered events, then `None`; a task that waits wakes.
#[test]
fn close_drains_the_buffer_then_ends() {
    let _serial = serial();
    let hub = Hub::with_default_executor();
    let mut stream = quotes(&hub);
    let waker = CountingWaker::new();
    assert!(poll(&mut stream, &waker).is_pending());
    publish_bids(&hub, "S0", &[1.0]);
    assert!(eventually(|| stream.buffered() == 1));
    stream.close().unwrap();
    stream.close().unwrap();
    assert!(stream.subscription().is_closed().unwrap());
    assert!(matches!(poll(&mut stream, &waker), Poll::Ready(Some(_))));
    assert!(matches!(poll(&mut stream, &waker), Poll::Ready(None)));

    let mut stream = quotes(&hub);
    let waker = CountingWaker::new();
    assert!(poll(&mut stream, &waker).is_pending());
    stream.close().unwrap();
    assert_eq!(waker.total(), 1);
    assert!(matches!(poll(&mut stream, &waker), Poll::Ready(None)));
}

/// Closing a stream drops the events that the SDK holds for it, and counts them.
#[test]
fn close_counts_the_events_that_the_sdk_held() {
    let _serial = serial();
    let hub = Hub::with_default_executor();
    let mut stream = quotes(&hub);
    hub.publisher
        .publish_events(&[
            quote_with_bid("S0", 1.0).into(),
            quote_with_bid("S1", 1.0).into(),
            quote_with_bid("S2", 1.0).into(),
        ])
        .unwrap();
    settle();
    stream.close().unwrap();
    assert_eq!((stream.buffered(), stream.dropped()), (0, 3));
    assert!(matches!(next(&mut stream), Some(None)));
}

/// Closing the subscription through the stream ends the stream.
#[test]
fn closing_the_subscription_ends_the_stream() {
    let _serial = serial();
    let hub = Hub::with_default_executor();
    let mut stream = quotes(&hub);
    let waker = CountingWaker::new();
    assert!(poll(&mut stream, &waker).is_pending());
    stream.subscription().close().unwrap();
    assert!(matches!(next(&mut stream), Some(None)));
    assert!(waker.total() <= 1);
}

/// Closing the endpoint ends the stream after its buffered events.
#[test]
fn closing_the_endpoint_ends_the_stream() {
    let _serial = serial();
    let hub = Hub::with_default_executor();
    let mut stream = quotes(&hub);
    assert!(poll(&mut stream, &CountingWaker::new()).is_pending());
    publish_bids(&hub, "S0", &[1.0]);
    assert!(eventually(|| stream.buffered() == 1));
    hub.endpoint.close().unwrap();
    assert_eq!(next_quote(&mut stream).symbol, "S0");
    assert!(matches!(next(&mut stream), Some(None)));
}

/// `close_and_await_termination` does not wait for a consumer: the stream takes the events that its queue holds.
#[test]
fn awaited_close_takes_the_events_that_the_queue_holds() {
    let _serial = serial();
    let hub = Hub::with_default_executor();
    let mut stream = quotes(&hub);
    publish_bids(&hub, "S0", &[1.0, 2.0]);
    publish_bids(&hub, "S1", &[1.0]);
    let (sender, terminated) = mpsc::channel();
    let endpoint = hub.endpoint;
    thread::spawn(move || sender.send(endpoint.close_and_await_termination().is_ok()));
    assert!(
        terminated.recv_timeout(LONG).unwrap(),
        "the endpoint waits for the events of the queue"
    );
    let mut bids: Vec<_> = (0..2)
        .map(|_| next_quote(&mut stream))
        .map(|q| (q.symbol, q.bid_price))
        .collect();
    bids.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(bids, [("S0".to_owned(), 2.0), ("S1".to_owned(), 1.0)]);
    assert!(matches!(next(&mut stream), Some(None)));
}

/// A subscription attached to two endpoints ends its stream once both are closed; another endpoint does not matter.
#[test]
fn stream_ends_when_all_its_endpoints_close() {
    let _serial = serial();
    let (first, second, other) = (
        Hub::with_default_executor(),
        Hub::with_default_executor(),
        Hub::with_default_executor(),
    );
    let subscription = DXFeedSubscription::<Quote>::new().unwrap();
    subscription.attach(&first.feed).unwrap();
    subscription.attach(&second.feed).unwrap();
    let mut stream = subscription.into_stream_with(StreamOptions::event_queue()).unwrap();
    let mut detached = DXFeedSubscription::<Quote>::new()
        .unwrap()
        .into_stream_with(StreamOptions::event_queue())
        .unwrap();
    let waker = CountingWaker::new();
    other.endpoint.close().unwrap();
    first.endpoint.close().unwrap();
    assert!(poll(&mut stream, &waker).is_pending());
    assert!(poll(&mut detached, &waker).is_pending());
    second.endpoint.close().unwrap();
    assert!(matches!(next(&mut stream), Some(None)));
    settle();
    assert!(poll(&mut detached, &waker).is_pending());
}

/// The awaited close of one of two endpoints takes the held events, and the stream conflates on demand again after it.
#[test]
fn awaited_close_of_one_endpoint_keeps_the_stream() {
    let _serial = serial();
    let (first, second) = (Hub::with_default_executor(), Hub::with_default_executor());
    let subscription = DXFeedSubscription::<Quote>::new().unwrap();
    subscription.attach(&first.feed).unwrap();
    subscription.attach(&second.feed).unwrap();
    let mut stream = subscription.into_stream_with(StreamOptions::event_queue()).unwrap();
    stream.subscription().add_symbols(["S0", "S1"]).unwrap();
    publish_bids(&first, "S0", &[1.0, 2.0]);
    let (sender, terminated) = mpsc::channel();
    let endpoint = first.endpoint;
    thread::spawn(move || sender.send(endpoint.close_and_await_termination().is_ok()));
    assert!(terminated.recv_timeout(LONG).unwrap());
    let quote = next_quote(&mut stream);
    assert_eq!((quote.symbol.as_str(), quote.bid_price), ("S0", 2.0));

    publish_bids(&second, "S1", &[1.0, 2.0, 3.0]);
    if stream.buffered() != 0 {
        let state = format!("{stream:?}");
        let taken: Vec<_> = (0..stream.buffered())
            .map(|_| poll(&mut stream, &CountingWaker::new()))
            .collect();
        panic!("taken from the SDK with no request: {state}, {taken:?}");
    }
    let quote = next_quote(&mut stream);
    assert_eq!((quote.symbol.as_str(), quote.bid_price), ("S1", 3.0));
}

/// A waker that drops the stream, inside the wake on the thread of the stream.
struct Dropping(Mutex<Option<EventStream<Quote>>>);

impl Wake for Dropping {
    fn wake(self: Arc<Self>) {
        drop(self.0.lock().unwrap().take());
    }
}

#[test]
fn dropping_the_stream_in_the_wake() {
    let _serial = serial();
    let hub = Hub::with_default_executor();
    let (handles, events) = (live_handles(), live_events());
    let dropping = Arc::new(Dropping(Mutex::new(Some(quotes(&hub)))));
    let waker = Waker::from(dropping.clone());
    let polled = dropping
        .0
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .poll_next(&mut Context::from_waker(&waker));
    assert!(polled.is_pending());
    publish_bids(&hub, "S0", &[1.0]);
    assert!(eventually(|| dropping.0.lock().unwrap().is_none()));
    // The thread of the stream exits and lets go of the queue.
    assert!(
        eventually(|| (live_handles(), live_events()) == (handles, events)),
        "{:?} != {:?}",
        (live_handles(), live_events()),
        (handles, events)
    );
    publish_bids(&hub, "S1", &[1.0]);
}

/// Dropping a stream lets go of its subscription, its queue, its thread and its events.
#[test]
fn drop_releases_everything() {
    let _serial = serial();
    let hub = Hub::with_default_executor();
    let (handles, events) = (live_handles(), live_events());
    let mut stream = quotes(&hub);
    hub.publisher
        .publish_events(&[quote_with_bid("S0", 1.0).into(), quote_with_bid("S1", 1.0).into()])
        .unwrap();
    assert_eq!(next_quote(&mut stream).bid_price, 1.0);
    assert_eq!(next_quote(&mut stream).bid_price, 1.0);
    // A task that waits keeps the thread in a poll.
    assert!(poll(&mut stream, &CountingWaker::new()).is_pending());
    drop(stream);
    assert!(
        eventually(|| (live_handles(), live_events()) == (handles, events)),
        "{:?} != {:?}",
        (live_handles(), live_events()),
        (handles, events)
    );
}
