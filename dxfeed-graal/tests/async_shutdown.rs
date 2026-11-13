//! Shutdown, the futures of promises and the event streams. The scenario runs in a child process ([`support::run_child`]), because
//! shutdown is process-wide.

mod support;

use std::future::IntoFuture;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::thread;
use std::time::{Duration, Instant};

use dxfeed_graal::event::{Order, OrderSource, Quote, Side};
use dxfeed_graal::event_queue::SharedEventQueue;
use dxfeed_graal::model::{IndexedTxModel, MarketDepthModel};
use dxfeed_graal::{Error, ErrorManager, EventStream, Promise, Promises, StreamOptions, system};
use support::{CountingWaker, Hub, block_on_with, is_child, poll_with, quote, quote_with_bid, run_child};

/// A buy order of the NTV book of `TX`, a transaction of its own.
fn order(index: i64) -> Order {
    let mut order = Order {
        index,
        price: 100.0,
        size: 1.0,
        ..Order::new("TX")
    };
    order.set_source(OrderSource::NTV).unwrap();
    order.set_order_side(Side::Buy);
    order
}

/// A stream of quotes over the event queue of its subscription.
fn queue_stream(hub: &Hub) -> EventStream<Quote> {
    hub.feed
        .create_subscription::<Quote>()
        .unwrap()
        .into_stream_with(StreamOptions::event_queue())
        .unwrap()
}

/// Polls `stream` once with `waker`.
fn poll_stream(stream: &mut EventStream<Quote>, waker: &Arc<CountingWaker>) -> Poll<Option<Quote>> {
    let waker = Waker::from(Arc::clone(waker));
    stream.poll_next(&mut Context::from_waker(&waker))
}

#[test]
fn shutdown_ends_pending_futures() {
    run_child("child_shutdown_ends_pending_futures");
}

#[test]
#[ignore = "run by shutdown_ends_pending_futures"]
fn child_shutdown_ends_pending_futures() {
    if !is_child() {
        return;
    }
    let hub = Hub::with_default_executor();
    let waker = CountingWaker::new();
    let mut request = hub.feed.get_last_event_promise::<Quote>("AAPL").unwrap().into_future();
    let mut any_of = Promises::any_of(vec![Promise::<Quote>::new().unwrap()])
        .unwrap()
        .into_future();
    assert!(poll_with(&mut request, &waker).is_pending());
    assert!(poll_with(&mut any_of, &waker).is_pending());
    let done = Promise::<()>::new().unwrap();
    done.complete().unwrap();
    let mut done = done.into_future();
    let late = Promise::<()>::new().unwrap();
    // A live stream with a buffered event, and one waiting for events.
    let mut quotes = hub.feed.create_subscription::<Quote>().unwrap().into_stream().unwrap();
    quotes.subscription().add_symbols(["IBM"]).unwrap();
    let mut waiting = hub.feed.create_subscription::<Quote>().unwrap().into_stream().unwrap();
    hub.publisher.publish_events(&[quote("IBM").into()]).unwrap();
    let stream_waker = CountingWaker::new();
    let first = block_on_with(quotes.next(), &stream_waker, Duration::from_secs(10));
    assert!(matches!(first, Some(Some(_))), "{first:?}");
    hub.publisher
        .publish_events(&[quote_with_bid("IBM", 2.0).into()])
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while quotes.buffered() == 0 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(quotes.buffered(), 1);
    assert!(poll_stream(&mut waiting, &stream_waker).is_pending());
    // The same over event queues: the thread of the waiting one is in a blocking poll.
    let mut queued = queue_stream(&hub);
    queued.subscription().add_symbols(["MSFT"]).unwrap();
    let mut queue_waiting = queue_stream(&hub);
    hub.publisher
        .publish_events(&[quote_with_bid("MSFT", 3.0).into()])
        .unwrap();
    assert!(poll_stream(&mut queued, &stream_waker).is_pending());
    let deadline = Instant::now() + Duration::from_secs(10);
    while queued.buffered() == 0 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(queued.buffered(), 1);
    assert!(poll_stream(&mut queue_waiting, &stream_waker).is_pending());
    // A stream of a shared queue, waiting.
    let mut tagged = SharedEventQueue::new().unwrap().into_stream().unwrap();
    let tagged_quotes = hub.feed.create_subscription::<Quote>().unwrap();
    tagged.queue().attach(&tagged_quotes, 1).unwrap();
    let polled = tagged.poll_next(&mut Context::from_waker(&Waker::from(Arc::clone(&stream_waker))));
    assert!(polled.is_pending());
    // A model stream with a buffered transaction, and a book stream waiting for a book.
    let mut orders = IndexedTxModel::<Order>::new_builder()
        .with_feed(&hub.feed)
        .with_symbol("TX")
        .build_stream()
        .unwrap();
    let mut depth = MarketDepthModel::<Order>::new_builder()
        .with_feed(&hub.feed)
        .with_symbol("DEPTH")
        .build_stream()
        .unwrap();
    hub.publisher.publish_events(&[order(1).into()]).unwrap();
    let first = block_on_with(orders.next(), &stream_waker, Duration::from_secs(10));
    assert!(matches!(first, Some(Some(_))), "{first:?}");
    hub.publisher.publish_events(&[order(2).into()]).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while orders.buffered() == 0 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(orders.buffered(), 1);
    let depth_waker = CountingWaker::new();
    let polled = depth.poll_next(&mut Context::from_waker(&Waker::from(Arc::clone(&depth_waker))));
    assert!(polled.is_pending());
    ErrorManager::clear_errors();
    // Futures made on another thread while shutdown runs.
    let racing: Vec<Promise<()>> = (0..200).map(|_| Promise::new().unwrap()).collect();
    let racer = thread::spawn(move || racing.into_iter().map(IntoFuture::into_future).collect::<Vec<_>>());

    system::shutdown().unwrap();

    assert_eq!(waker.total(), 2);
    // The streams end: the buffered event first.
    assert!(matches!(poll_stream(&mut quotes, &stream_waker), Poll::Ready(Some(_))));
    assert!(matches!(poll_stream(&mut quotes, &stream_waker), Poll::Ready(None)));
    assert!(matches!(poll_stream(&mut waiting, &stream_waker), Poll::Ready(None)));
    // The streams over event queues end on their threads.
    let long = Duration::from_secs(10);
    assert!(matches!(poll_stream(&mut queued, &stream_waker), Poll::Ready(Some(_))));
    assert!(matches!(block_on_with(queued.next(), &stream_waker, long), Some(None)));
    assert!(matches!(
        block_on_with(queue_waiting.next(), &stream_waker, long),
        Some(None)
    ));
    assert!(matches!(block_on_with(tagged.next(), &stream_waker, long), Some(None)));
    drop((tagged, tagged_quotes));
    // The model streams end, the buffered transaction first.
    let mut cx = Context::from_waker(Waker::noop());
    assert!(matches!(orders.poll_next(&mut cx), Poll::Ready(Some(_))));
    assert!(matches!(orders.poll_next(&mut cx), Poll::Ready(None)));
    assert_eq!(depth_waker.total(), 1);
    assert!(matches!(depth.poll_next(&mut cx), Poll::Ready(None)));
    drop((orders, depth));
    let errors = ErrorManager::get_errors();
    assert!(
        errors.iter().all(|error| {
            !["DXFeedSubscription", "EventQueue", "TxModel", "MarketDepthModel"].contains(&error.component.as_str())
        }),
        "{errors:?}"
    );
    assert!(matches!(
        poll_with(&mut request, &waker),
        Poll::Ready(Err(Error::ShutDown))
    ));
    assert!(matches!(
        poll_with(&mut any_of, &waker),
        Poll::Ready(Err(Error::ShutDown))
    ));
    // A future that completed before shutdown keeps its result.
    assert!(matches!(poll_with(&mut done, &waker), Poll::Ready(Ok(()))));
    // A future made after shutdown is ready at once.
    assert!(matches!(
        poll_with(&mut late.into_future(), &waker),
        Poll::Ready(Err(Error::ShutDown))
    ));
    for future in racer.join().unwrap() {
        let result = block_on_with(future, &CountingWaker::new(), Duration::from_secs(10));
        assert!(matches!(result, Some(Err(Error::ShutDown))), "{result:?}");
    }
}
