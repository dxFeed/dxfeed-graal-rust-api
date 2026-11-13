//! Event streams over `LOCAL_HUB` endpoints: the buffer, the wakes and the end of a stream.

mod support;

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll, Wake, Waker};
use std::time::Duration;

use dxfeed_graal::event::{Candle, Quote, SubscriptionItem, TimeAndSale};
use dxfeed_graal::{
    DXFeedSubscription, DXFeedTimeSeriesSubscription, Error, Event, EventStream, EventType, JavaExceptionClass,
    StreamOptions, live_events, live_handles,
};
use support::{CountingWaker, Hub, block_on_with, delivered, quote, quote_with_bid, same_events, time_and_sale, trade};

/// Runs the tests of this file one by one, so that each can count the live handles.
fn serial() -> MutexGuard<'static, ()> {
    static SERIAL: Mutex<()> = Mutex::new(());
    SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
}

const LONG: Duration = Duration::from_secs(10);

/// Polls `stream` once with `waker`.
fn poll<E, S>(stream: &mut EventStream<E, S>, waker: &Arc<CountingWaker>) -> Poll<Option<E>>
where
    E: SubscriptionItem,
    S: AsRef<DXFeedSubscription<E>>,
{
    let waker = Waker::from(Arc::clone(waker));
    stream.poll_next(&mut Context::from_waker(&waker))
}

/// The symbols of the quotes that `stream` yields until it is pending.
fn take_symbols(stream: &mut EventStream<Quote>, waker: &Arc<CountingWaker>) -> Vec<String> {
    let mut symbols = Vec::new();
    while let Poll::Ready(Some(quote)) = poll(stream, waker) {
        symbols.push(quote.symbol);
    }
    symbols
}

fn quotes(hub: &Hub, options: StreamOptions) -> EventStream<Quote> {
    let stream = hub
        .feed
        .create_subscription::<Quote>()
        .unwrap()
        .into_stream_with(options)
        .unwrap();
    stream
        .subscription()
        .add_symbols(["S0", "S1", "S2", "S3", "S4"])
        .unwrap();
    stream
}

#[test]
fn streams_are_send_sync_unpin_and_static() {
    fn check<T: Send + Sync + Unpin + 'static>() {}
    check::<EventStream<Quote>>();
    check::<EventStream<Event>>();
    check::<EventStream<Candle, DXFeedTimeSeriesSubscription<Candle>>>();
}

/// The events come in order as owned events, one wake per batch, and only once the executor runs.
#[test]
fn stream_yields_owned_events_in_order() {
    let _serial = serial();
    let hub = Hub::new();
    let mut stream = hub.feed.create_subscription::<Quote>().unwrap().into_stream().unwrap();
    stream.subscription().add_symbols(["AAPL"]).unwrap();
    let waker = CountingWaker::new();
    assert!(poll(&mut stream, &waker).is_pending());

    hub.publisher
        .publish_events(&[quote_with_bid("AAPL", 1.0).into()])
        .unwrap();
    assert!(poll(&mut stream, &waker).is_pending(), "the executor has not run");
    hub.executor.process_all_pending().unwrap();
    assert_eq!(waker.wakes(), 1);
    for bid in [2.0, 3.0] {
        hub.publish_and_process(&[quote_with_bid("AAPL", bid).into()]);
    }
    // The waker was taken by the first wake and not stored again.
    assert_eq!(waker.total(), 1);
    let bids: Vec<f64> = (0..3)
        .map(|_| match poll(&mut stream, &waker) {
            Poll::Ready(Some(quote)) => quote.bid_price,
            other => panic!("not an event: {other:?}"),
        })
        .collect();
    assert_eq!(bids, [1.0, 2.0, 3.0]);
    assert!(poll(&mut stream, &waker).is_pending());
    assert_eq!((stream.buffered(), stream.dropped(), stream.skipped()), (0, 0, 0));
}

/// Each batch wakes a waiting task once; a batch that comes while nothing waits wakes nothing.
#[test]
fn one_wake_per_batch() {
    let _serial = serial();
    let hub = Hub::new();
    let mut stream = quotes(&hub, StreamOptions::default());
    let waker = CountingWaker::new();
    assert!(poll(&mut stream, &waker).is_pending());
    hub.publish_and_process(&[quote("S0").into(), quote("S1").into(), quote("S2").into()]);
    assert_eq!(waker.total(), 1);
    hub.publish_and_process(&[quote("S3").into()]);
    assert_eq!(waker.total(), 1, "no task waits");
    assert_eq!(take_symbols(&mut stream, &waker).len(), 4);
    hub.publish_and_process(&[quote("S4").into()]);
    assert_eq!(waker.total(), 2);
}

/// A full buffer drops the oldest events and counts them.
#[test]
fn full_buffer_drops_the_oldest() {
    let _serial = serial();
    let hub = Hub::new();
    let mut stream = quotes(&hub, StreamOptions::bounded(2));
    for symbol in ["S0", "S1", "S2", "S3", "S4"] {
        hub.publish_and_process(&[quote(symbol).into()]);
    }
    assert_eq!((stream.buffered(), stream.dropped()), (2, 3));
    assert_eq!(take_symbols(&mut stream, &CountingWaker::new()), ["S3", "S4"]);
    assert_eq!(stream.dropped(), 3);
}

/// An unbounded buffer keeps every event.
#[test]
fn unbounded_buffer_keeps_everything() {
    let _serial = serial();
    let hub = Hub::new();
    let mut stream = quotes(&hub, StreamOptions::unbounded());
    for symbol in ["S0", "S1", "S2", "S3", "S4"] {
        hub.publish_and_process(&[quote(symbol).into()]);
    }
    assert_eq!(
        take_symbols(&mut stream, &CountingWaker::new()),
        ["S0", "S1", "S2", "S3", "S4"]
    );
    assert_eq!(stream.dropped(), 0);
}

#[test]
fn options() {
    assert_eq!(StreamOptions::default(), StreamOptions::bounded(65_536));
    assert_eq!(
        StreamOptions::default().capacity(),
        Some(StreamOptions::DEFAULT_CAPACITY)
    );
    assert_eq!(StreamOptions::unbounded().capacity(), None);
    let _serial = serial();
    let hub = Hub::new();
    let result = hub
        .feed
        .create_subscription::<Quote>()
        .unwrap()
        .into_stream_with(StreamOptions::bounded(0));
    assert!(matches!(result, Err(Error::InvalidArgument(_))), "{result:?}");
}

/// An untyped stream yields the events of each of its types as [`Event`].
#[test]
fn untyped_stream_yields_events() {
    let _serial = serial();
    let hub = Hub::new();
    let mut stream = hub
        .feed
        .create_subscription_with_types(&[EventType::Quote, EventType::Trade])
        .unwrap()
        .into_stream()
        .unwrap();
    stream.subscription().add_symbols(["AAPL"]).unwrap();
    hub.publish_and_process(&[quote("AAPL").into()]);
    hub.publish_and_process(&[trade("AAPL").into()]);
    let waker = CountingWaker::new();
    let events: Vec<Event> = std::iter::from_fn(|| match poll(&mut stream, &waker) {
        Poll::Ready(event) => event,
        Poll::Pending => None,
    })
    .collect();
    assert_eq!(events.len(), 2);
    assert!(same_events(&events[0], &delivered(quote("AAPL").into())));
    assert!(same_events(&events[1], &delivered(trade("AAPL").into())));
    assert_eq!(stream.skipped(), 0);
}

/// A closed stream yields its buffered events, then `None` for good, and discards later events.
#[test]
fn close_drains_then_ends() {
    let _serial = serial();
    let hub = Hub::new();
    let mut stream = quotes(&hub, StreamOptions::default());
    hub.publish_and_process(&[quote("S0").into(), quote("S1").into()]);
    let waker = CountingWaker::new();
    stream.close().unwrap();
    stream.close().unwrap();
    assert!(stream.subscription().is_closed().unwrap());
    hub.publish_and_process(&[quote("S2").into()]);
    assert_eq!(take_symbols(&mut stream, &waker), ["S0", "S1"]);
    assert!(matches!(poll(&mut stream, &waker), Poll::Ready(None)));
    assert!(matches!(poll(&mut stream, &waker), Poll::Ready(None)));
}

/// A task waiting on a stream wakes when the stream ends.
#[test]
fn close_wakes_a_waiting_task() {
    let _serial = serial();
    let hub = Hub::new();
    let mut stream = quotes(&hub, StreamOptions::default());
    let waker = CountingWaker::new();
    assert!(poll(&mut stream, &waker).is_pending());
    stream.close().unwrap();
    assert_eq!(waker.total(), 1);
    assert!(matches!(poll(&mut stream, &waker), Poll::Ready(None)));
}

/// Closing the subscription through the stream ends the stream.
#[test]
fn closing_the_subscription_ends_the_stream() {
    let _serial = serial();
    let hub = Hub::with_default_executor();
    let mut stream = quotes(&hub, StreamOptions::default());
    stream.subscription().close().unwrap();
    let result = block_on_with(stream.next(), &CountingWaker::new(), LONG);
    assert!(matches!(result, Some(None)), "{result:?}");
}

/// Closing the endpoint, with or without awaiting termination, ends the stream.
#[test]
fn closing_the_endpoint_ends_the_stream() {
    let _serial = serial();
    for await_termination in [false, true] {
        let hub = Hub::with_default_executor();
        let mut stream = quotes(&hub, StreamOptions::default());
        let waker = CountingWaker::new();
        assert!(poll(&mut stream, &waker).is_pending());
        if await_termination {
            hub.endpoint.close_and_await_termination().unwrap();
        } else {
            hub.endpoint.close().unwrap();
        }
        assert_eq!(waker.total(), 1, "await_termination={await_termination}");
        assert!(matches!(poll(&mut stream, &waker), Poll::Ready(None)));
    }
}

/// A subscription attached to two endpoints ends its stream once both are closed; another endpoint does not matter.
#[test]
fn stream_ends_when_all_its_endpoints_close() {
    let _serial = serial();
    let (first, second, other) = (Hub::new(), Hub::new(), Hub::new());
    let subscription = DXFeedSubscription::<Quote>::new().unwrap();
    subscription.attach(&first.feed).unwrap();
    subscription.attach(&second.feed).unwrap();
    let mut stream = subscription.into_stream().unwrap();
    let mut detached = DXFeedSubscription::<Quote>::new().unwrap().into_stream().unwrap();
    let waker = CountingWaker::new();
    other.endpoint.close().unwrap();
    first.endpoint.close().unwrap();
    assert!(poll(&mut stream, &waker).is_pending());
    assert!(poll(&mut detached, &waker).is_pending());
    second.endpoint.close().unwrap();
    assert!(matches!(poll(&mut stream, &waker), Poll::Ready(None)));
    assert!(poll(&mut detached, &waker).is_pending());
}

/// A stream of a closed subscription, or of a closed endpoint, ends at once.
#[test]
fn stream_of_a_closed_subscription_ends_at_once() {
    let _serial = serial();
    let hub = Hub::new();
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    subscription.close().unwrap();
    let mut stream = subscription.into_stream().unwrap();
    assert!(matches!(poll(&mut stream, &CountingWaker::new()), Poll::Ready(None)));

    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    hub.endpoint.close().unwrap();
    let mut stream = subscription.into_stream().unwrap();
    assert!(matches!(poll(&mut stream, &CountingWaker::new()), Poll::Ready(None)));
}

/// The SDK refuses the listener of a subscription that has symbols.
#[test]
fn stream_needs_a_subscription_without_symbols() {
    let _serial = serial();
    let hub = Hub::new();
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    subscription.add_symbols(["AAPL"]).unwrap();
    match subscription.into_stream() {
        Err(Error::Java(exception)) => assert_eq!(
            exception.get_class_name(),
            JavaExceptionClass::IllegalStateException.get_name()
        ),
        other => panic!("not the refusal: {other:?}"),
    }
}

/// A time-series stream keeps the time of its subscription.
#[test]
fn time_series_stream() {
    let _serial = serial();
    let hub = Hub::new();
    let mut stream = hub
        .feed
        .create_time_series_subscription::<TimeAndSale>()
        .unwrap()
        .into_stream()
        .unwrap();
    stream.subscription().set_from_time(0).unwrap();
    assert_eq!(stream.subscription().get_from_time(), 0);
    stream.subscription().add_symbols(["AAPL"]).unwrap();
    hub.publish_and_process(&[time_and_sale("AAPL").into()]);
    match poll(&mut stream, &CountingWaker::new()) {
        Poll::Ready(Some(sale)) => assert!(same_events(
            &Event::from(sale),
            &delivered(time_and_sale("AAPL").into())
        )),
        other => panic!("not an event: {other:?}"),
    }
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
    let hub = Hub::new();
    let stream = Arc::new(Mutex::new(Some(quotes(&hub, StreamOptions::default()))));
    let waker = Waker::from(Arc::new(Reentrant(stream.clone())));
    let polled = stream
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .poll_next(&mut Context::from_waker(&waker));
    assert!(polled.is_pending());
    hub.publish_and_process(&[quote("S0").into()]);
    assert_eq!(stream.lock().unwrap().as_ref().unwrap().buffered(), 1);

    // The end of the stream wakes outside the lock as well.
    let poll_again = || {
        stream
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .poll_next(&mut Context::from_waker(&waker))
    };
    assert!(matches!(poll_again(), Poll::Ready(Some(_))));
    assert!(poll_again().is_pending());
    hub.endpoint.close().unwrap();
    assert!(matches!(poll_again(), Poll::Ready(None)));
}

/// The events scheduled before `close_and_await_termination` are in the buffer when it returns.
#[test]
fn awaited_close_keeps_scheduled_events() {
    let _serial = serial();
    let hub = Hub::with_default_executor();
    let mut stream = quotes(&hub, StreamOptions::default());
    hub.publisher.publish_events(&[quote("S0").into()]).unwrap();
    hub.endpoint.close_and_await_termination().unwrap();
    let waker = CountingWaker::new();
    assert_eq!(take_symbols(&mut stream, &waker), ["S0"]);
    assert!(matches!(poll(&mut stream, &waker), Poll::Ready(None)));
}

/// A waker that drops the stream, inside the listener call that wakes it.
struct Dropping(Mutex<Option<EventStream<Quote>>>);

impl Wake for Dropping {
    fn wake(self: Arc<Self>) {
        drop(self.0.lock().unwrap().take());
    }
}

#[test]
fn dropping_the_stream_in_the_wake() {
    let _serial = serial();
    let hub = Hub::new();
    let dropping = Arc::new(Dropping(Mutex::new(Some(quotes(&hub, StreamOptions::default())))));
    let waker = Waker::from(dropping.clone());
    let polled = dropping
        .0
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .poll_next(&mut Context::from_waker(&waker));
    assert!(polled.is_pending());
    hub.publish_and_process(&[quote("S0").into()]);
    assert!(dropping.0.lock().unwrap().is_none());
    hub.publish_and_process(&[quote("S1").into()]);
}

/// Dropping a stream lets go of its subscription, its listeners and its events.
#[test]
fn drop_releases_everything() {
    let _serial = serial();
    let hub = Hub::new();
    let (handles, events) = (live_handles(), live_events());
    let mut stream = quotes(&hub, StreamOptions::default());
    hub.publish_and_process(&[quote("S0").into(), quote("S1").into()]);
    assert!(matches!(poll(&mut stream, &CountingWaker::new()), Poll::Ready(Some(_))));
    drop(stream);
    hub.executor.process_all_pending().unwrap();
    assert_eq!((live_handles(), live_events()), (handles, events));
}

/// A stream on another thread gets the events of the endpoint's own executor.
#[test]
fn stream_on_the_default_executor() {
    let _serial = serial();
    let hub = Hub::with_default_executor();
    let mut stream = quotes(&hub, StreamOptions::default());
    hub.publisher.publish_events(&[quote("S2").into()]).unwrap();
    let result = block_on_with(stream.next(), &CountingWaker::new(), LONG);
    assert!(matches!(&result, Some(Some(q)) if q.symbol == "S2"), "{result:?}");
}

#[cfg(feature = "stream")]
#[test]
fn stream_trait() {
    use std::pin::Pin;

    use futures_core::Stream;

    let _serial = serial();
    let hub = Hub::new();
    let mut stream = quotes(&hub, StreamOptions::default());
    assert_eq!(Stream::size_hint(&stream), (0, None));
    hub.publish_and_process(&[quote("S0").into(), quote("S1").into()]);
    assert_eq!(Stream::size_hint(&stream), (2, None));
    let waker = Waker::from(CountingWaker::new());
    let mut cx = Context::from_waker(&waker);
    assert!(matches!(
        Stream::poll_next(Pin::new(&mut stream), &mut cx),
        Poll::Ready(Some(_))
    ));
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
    assert_eq!(Stream::size_hint(&stream), (0, Some(0)));
}
