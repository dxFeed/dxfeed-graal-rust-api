//! Streams of shared event queues ([`SharedEventQueue::into_stream`]) on `LOCAL_HUB` endpoints: the tags, the
//! conflation on demand, the work of the executor of the queue, the wakes and the end of a stream.

mod support;

use std::sync::{Arc, Mutex, MutexGuard, PoisonError, mpsc};
use std::task::{Context, Poll, Wake, Waker};
use std::thread;
use std::time::{Duration, Instant};

use dxfeed_graal::event::{Candle, Quote, Trade};
use dxfeed_graal::event_queue::{PollResult, SharedEventQueue};
use dxfeed_graal::model::{ObservableListModel, TimeSeriesEventModel};
use dxfeed_graal::{DXFeedSubscription, Event, EventType, Promise, TaggedEventStream, live_events, live_handles};
use support::{CountingWaker, Hub, block_on_with, candle, quote_with_bid, trade};

/// Runs the tests of this file one by one, so that each can count the live handles.
fn serial() -> MutexGuard<'static, ()> {
    static SERIAL: Mutex<()> = Mutex::new(());
    SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
}

const LONG: Duration = Duration::from_secs(10);

/// Lets the hub deliver what was published to the subscriptions, so that the queue holds it.
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
fn poll(stream: &mut TaggedEventStream, waker: &Arc<CountingWaker>) -> Poll<Option<(i64, Event)>> {
    let waker = Waker::from(Arc::clone(waker));
    stream.poll_next(&mut Context::from_waker(&waker))
}

/// The next item of `stream`, waiting for it on this thread; `None` if nothing comes within `LONG`.
fn next(stream: &mut TaggedEventStream) -> Option<Option<(i64, Event)>> {
    block_on_with(stream.next(), &CountingWaker::new(), LONG)
}

/// The next event of `stream` with its tag, waiting for it.
fn next_event(stream: &mut TaggedEventStream) -> (i64, Event) {
    match next(stream) {
        Some(Some(item)) => item,
        other => panic!("no event: {other:?}"),
    }
}

/// The tag, the symbol and the bid of a quote.
fn bid((tag, event): (i64, Event)) -> (i64, String, f64) {
    match event {
        Event::Quote(quote) => (tag, quote.symbol, quote.bid_price),
        other => panic!("not a quote: {other:?}"),
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

/// A stream of a shared queue with the quotes of S0..S4 of `hub`, attached with the tag 1, and their subscription,
/// which the stream does not own: dropping it closes it.
fn quotes(hub: &Hub) -> (TaggedEventStream, DXFeedSubscription<Quote>) {
    let stream = SharedEventQueue::new().unwrap().into_stream().unwrap();
    let quotes = hub.feed.create_subscription::<Quote>().unwrap();
    stream.queue().attach(&quotes, 1).unwrap();
    quotes.add_symbols(["S0", "S1", "S2", "S3", "S4"]).unwrap();
    (stream, quotes)
}

// The delivery of the events.

/// The events of several subscriptions, attached before and after the stream is made, come with their tags.
#[test]
fn yields_the_events_of_its_subscriptions_with_their_tags() {
    let _serial = serial();
    let hub = Hub::with_default_executor();
    let queue = SharedEventQueue::new().unwrap();
    let quotes = hub.feed.create_subscription::<Quote>().unwrap();
    queue.attach(&quotes, 1).unwrap();
    let mut stream = queue.into_stream().unwrap();
    let trades = hub.feed.create_subscription::<Trade>().unwrap();
    stream.queue().attach(&trades, 2).unwrap();
    quotes.add_symbols(["S0"]).unwrap();
    trades.add_symbols(["S0"]).unwrap();
    hub.publisher
        .publish_events(&[quote_with_bid("S0", 1.0).into(), trade("S0").into()])
        .unwrap();
    let mut items: Vec<_> = (0..2)
        .map(|_| next_event(&mut stream))
        .map(|(tag, event)| (tag, event.event_type()))
        .collect();
    items.sort_by_key(|&(tag, _)| tag);
    assert_eq!(items, [(1, EventType::Quote), (2, EventType::Trade)]);
    assert_eq!((stream.buffered(), stream.dropped(), stream.skipped()), (0, 0, 0));
}

/// The quotes of a symbol published while no task waits come as the last one.
#[test]
fn conflates_the_quotes_published_while_no_task_waits() {
    let _serial = serial();
    let hub = Hub::with_default_executor();
    let (mut stream, _quotes) = quotes(&hub);
    publish_bids(&hub, "S0", &[1.0, 2.0, 3.0]);
    publish_bids(&hub, "S1", &[11.0]);
    let mut bids: Vec<_> = (0..2).map(|_| bid(next_event(&mut stream))).collect();
    bids.sort_by(|a, b| a.1.cmp(&b.1));
    assert_eq!(bids, [(1, "S0".to_owned(), 3.0), (1, "S1".to_owned(), 11.0)]);
    assert!(poll(&mut stream, &CountingWaker::new()).is_pending());
    assert_eq!(stream.buffered(), 0);
}

/// The work of the executor of the queue waits for a task that waits for events, and runs on the thread of the
/// stream.
#[test]
fn the_work_of_the_executor_runs_while_a_task_waits() {
    let _serial = serial();
    let mut stream = SharedEventQueue::new().unwrap().into_stream().unwrap();
    let promise = Promise::<()>::new().unwrap();
    let (sender, done) = mpsc::channel();
    promise
        .when_done_async(
            move |_| {
                let _ = sender.send(thread::current().name().map(str::to_owned));
            },
            &stream.queue().get_executor().unwrap(),
        )
        .unwrap();
    promise.complete().unwrap();
    assert!(
        done.recv_timeout(Duration::from_millis(300)).is_err(),
        "ran with no task waiting"
    );
    assert!(poll(&mut stream, &CountingWaker::new()).is_pending());
    let thread = done.recv_timeout(LONG).unwrap();
    assert_eq!(thread.as_deref(), Some("dxfeed-event-stream"));
    // The work brought no events: the task still waits.
    assert!(poll(&mut stream, &CountingWaker::new()).is_pending());
}

/// A model, its list of candles that has the listener, and what the listener sends.
type CandleModel = (
    TimeSeriesEventModel<Candle>,
    ObservableListModel<Candle>,
    mpsc::Receiver<(usize, Option<String>)>,
);

/// A model of daily candles of C on the executor of the queue of `stream`, attached to `hub`, whose listener sends the
/// count of candles and the name of its thread.
fn candle_model(hub: &Hub, stream: &TaggedEventStream) -> CandleModel {
    let model = TimeSeriesEventModel::<Candle>::new().unwrap();
    model
        .set_executor(Some(&stream.queue().get_executor().unwrap()))
        .unwrap();
    model.set_symbol(Some("C{=d}")).unwrap();
    model.set_from_time(1_000).unwrap();
    let (sender, changes) = mpsc::channel();
    let candles = model.get_events_list().unwrap();
    candles
        .add_listener(move |candles| {
            let _ = sender.send((candles.count(), thread::current().name().map(str::to_owned)));
        })
        .unwrap();
    model.attach(&hub.feed).unwrap();
    let mut published = candle("C{=d}");
    published.set_time(2_000);
    hub.publisher.publish_events(&[published.into()]).unwrap();
    (model, candles, changes)
}

/// A model on the executor of the queue notifies in the polls of the stream, while a task waits.
#[test]
fn a_model_on_the_executor_of_the_queue_notifies_through_the_stream() {
    let _serial = serial();
    let hub = Hub::with_default_executor();
    let mut stream = SharedEventQueue::new().unwrap().into_stream().unwrap();
    let (model, _candles, changes) = candle_model(&hub, &stream);
    assert!(
        changes.recv_timeout(Duration::from_millis(300)).is_err(),
        "notified with no task waiting"
    );
    assert!(poll(&mut stream, &CountingWaker::new()).is_pending());
    let thread = Some("dxfeed-event-stream".to_owned());
    assert_eq!(changes.recv_timeout(LONG).unwrap(), (1, thread));
    model.close().unwrap();
}

/// An endpoint that awaits its termination does not wait for a task: the stream runs the pending work of a model on
/// the executor of its queue.
#[test]
fn awaited_close_runs_the_pending_work_of_the_executor() {
    let _serial = serial();
    let hub = Hub::with_default_executor();
    let stream = SharedEventQueue::new().unwrap().into_stream().unwrap();
    let (model, _candles, changes) = candle_model(&hub, &stream);
    settle();
    let (sender, terminated) = mpsc::channel();
    let endpoint = hub.endpoint;
    thread::spawn(move || sender.send(endpoint.close_and_await_termination().is_ok()));
    assert!(
        terminated.recv_timeout(LONG).unwrap(),
        "the endpoint waits for the work of the executor"
    );
    let thread = Some("dxfeed-event-stream".to_owned());
    assert_eq!(changes.recv_timeout(LONG).unwrap(), (1, thread));
    model.close().unwrap();
}

#[test]
fn debug_and_bounds() {
    fn bounds<T: Send + Sync + Unpin + 'static>() {}
    bounds::<TaggedEventStream>();
    let _serial = serial();
    let hub = Hub::with_default_executor();
    let (stream, _quotes) = quotes(&hub);
    let debug = format!("{stream:?}");
    for field in ["buffered: 0", "closed: false", "dropped: 0", "skipped: 0"] {
        assert!(debug.contains(field), "{debug}");
    }
}

/// A waker that formats the stream, which locks its buffer: it hangs if the stream wakes under that lock.
struct Reentrant(Arc<Mutex<Option<TaggedEventStream>>>);

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
    let (stream, _quotes) = quotes(&hub);
    let stream = Arc::new(Mutex::new(Some(stream)));
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
    stream.lock().unwrap().as_ref().unwrap().queue().close().unwrap();
    assert!(eventually(|| matches!(poll_again(), Poll::Ready(None))));
}

#[cfg(feature = "stream")]
#[test]
fn stream_trait() {
    use std::pin::Pin;

    use futures_core::Stream;

    let _serial = serial();
    let hub = Hub::with_default_executor();
    let (mut stream, _quotes) = quotes(&hub);
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
        Poll::Ready(Some((1, _)))
    ));
    assert!(matches!(
        Stream::poll_next(Pin::new(&mut stream), &mut cx),
        Poll::Ready(None)
    ));
}

// The end of a stream.

/// A closed stream yields its buffered events, then `None`; a task that waits wakes; the events that the queue held
/// are counted.
#[test]
fn close_drains_the_buffer_then_ends() {
    let _serial = serial();
    let hub = Hub::with_default_executor();
    let (mut stream, _quotes) = quotes(&hub);
    let waker = CountingWaker::new();
    assert!(poll(&mut stream, &waker).is_pending());
    publish_bids(&hub, "S0", &[1.0]);
    assert!(eventually(|| stream.buffered() == 1));
    stream.close().unwrap();
    stream.close().unwrap();
    assert_eq!(
        stream.queue().poll(Some(Duration::ZERO), 0).unwrap(),
        PollResult::Closed
    );
    assert!(matches!(poll(&mut stream, &waker), Poll::Ready(Some((1, _)))));
    assert!(matches!(poll(&mut stream, &waker), Poll::Ready(None)));

    let (mut stream, _quotes) = quotes(&hub);
    let waker = CountingWaker::new();
    assert!(poll(&mut stream, &waker).is_pending());
    stream.close().unwrap();
    assert_eq!(waker.total(), 1);
    assert!(matches!(poll(&mut stream, &waker), Poll::Ready(None)));

    let (mut stream, _quotes) = quotes(&hub);
    hub.publisher
        .publish_events(&[quote_with_bid("S0", 1.0).into(), quote_with_bid("S1", 1.0).into()])
        .unwrap();
    settle();
    stream.close().unwrap();
    assert_eq!((stream.buffered(), stream.dropped()), (0, 2));
    assert!(matches!(next(&mut stream), Some(None)));
}

/// Closing the queue through the stream ends the stream, also a queue closed before the stream was made.
#[test]
fn closing_the_queue_ends_the_stream() {
    let _serial = serial();
    let hub = Hub::with_default_executor();
    let (mut stream, _quotes) = quotes(&hub);
    let waker = CountingWaker::new();
    assert!(poll(&mut stream, &waker).is_pending());
    stream.queue().close().unwrap();
    assert!(matches!(next(&mut stream), Some(None)));
    assert!(waker.total() <= 1);

    let queue = SharedEventQueue::new().unwrap();
    queue.close().unwrap();
    let mut stream = queue.into_stream().unwrap();
    assert!(matches!(next(&mut stream), Some(None)));
}

/// Closing an endpoint does not end the stream: the subscriptions of another endpoint still deliver.
#[test]
fn closing_an_endpoint_keeps_the_stream() {
    let _serial = serial();
    let (first, second) = (Hub::with_default_executor(), Hub::with_default_executor());
    let (mut stream, _quotes) = quotes(&first);
    let other = second.feed.create_subscription::<Quote>().unwrap();
    stream.queue().attach(&other, 2).unwrap();
    other.add_symbols(["S0"]).unwrap();
    first.endpoint.close().unwrap();
    settle();
    assert!(poll(&mut stream, &CountingWaker::new()).is_pending());
    publish_bids(&second, "S0", &[5.0]);
    assert_eq!(bid(next_event(&mut stream)), (2, "S0".to_owned(), 5.0));
}

/// `close_and_await_termination` does not wait for a consumer: the stream takes the events that its queue holds, and
/// conflates on demand again after it.
#[test]
fn awaited_close_takes_the_events_that_the_queue_holds() {
    let _serial = serial();
    let (first, second) = (Hub::with_default_executor(), Hub::with_default_executor());
    let (mut stream, _quotes) = quotes(&first);
    let other = second.feed.create_subscription::<Quote>().unwrap();
    stream.queue().attach(&other, 2).unwrap();
    other.add_symbols(["S1"]).unwrap();
    publish_bids(&first, "S0", &[1.0, 2.0]);
    let (sender, terminated) = mpsc::channel();
    let endpoint = first.endpoint;
    thread::spawn(move || sender.send(endpoint.close_and_await_termination().is_ok()));
    assert!(
        terminated.recv_timeout(LONG).unwrap(),
        "the endpoint waits for the events of the queue"
    );
    assert_eq!(bid(next_event(&mut stream)), (1, "S0".to_owned(), 2.0));
    // The last poll of the drain may still run for a moment.
    settle();

    publish_bids(&second, "S1", &[1.0, 2.0, 3.0]);
    assert_eq!(stream.buffered(), 0, "taken from the SDK with no request: {stream:?}");
    assert_eq!(bid(next_event(&mut stream)), (2, "S1".to_owned(), 3.0));
}

/// A waker that drops the stream, inside the wake on the thread of the stream.
struct Dropping(Mutex<Option<TaggedEventStream>>);

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
    let (stream, subscription) = quotes(&hub);
    let dropping = Arc::new(Dropping(Mutex::new(Some(stream))));
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
    drop(subscription);
    // The thread of the stream exits and lets go of the queue.
    assert!(
        eventually(|| (live_handles(), live_events()) == (handles, events)),
        "{:?} != {:?}",
        (live_handles(), live_events()),
        (handles, events)
    );
    publish_bids(&hub, "S1", &[1.0]);
}

/// Dropping a stream closes its queue, also for the executors it lent: they run the work at once, without calling the
/// listeners of the application.
#[test]
fn dropping_the_stream_closes_the_queue_for_its_executors() {
    let _serial = serial();
    let stream = SharedEventQueue::new().unwrap().into_stream().unwrap();
    let executor = stream.queue().get_executor().unwrap();
    drop(stream);
    let promise = Promise::<()>::new().unwrap();
    let (sender, done) = mpsc::channel();
    promise
        .when_done_async(move |_| sender.send(()).unwrap(), &executor)
        .unwrap();
    promise.complete().unwrap();
    assert!(done.recv_timeout(Duration::from_millis(300)).is_err());
}

/// Dropping a stream lets go of its queue, its thread and its events.
#[test]
fn drop_releases_everything() {
    let _serial = serial();
    let hub = Hub::with_default_executor();
    let (handles, events) = (live_handles(), live_events());
    let (mut stream, subscription) = quotes(&hub);
    hub.publisher
        .publish_events(&[quote_with_bid("S0", 1.0).into(), quote_with_bid("S1", 1.0).into()])
        .unwrap();
    assert_eq!(bid(next_event(&mut stream)).2, 1.0);
    assert_eq!(bid(next_event(&mut stream)).2, 1.0);
    // A task that waits keeps the thread in a poll.
    assert!(poll(&mut stream, &CountingWaker::new()).is_pending());
    drop(stream);
    drop(subscription);
    assert!(
        eventually(|| (live_handles(), live_events()) == (handles, events)),
        "{:?} != {:?}",
        (live_handles(), live_events()),
        (handles, events)
    );
}
