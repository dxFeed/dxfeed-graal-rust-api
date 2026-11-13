//! Shutdown, the futures of promises and the event streams. The scenario runs in a child process ([`support::run_child`]), because
//! shutdown is process-wide.

mod support;

use std::future::IntoFuture;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::thread;
use std::time::{Duration, Instant};

use dxfeed_graal::event::Quote;
use dxfeed_graal::{Error, EventStream, Promise, Promises, system};
use support::{CountingWaker, Hub, block_on_with, is_child, poll_with, quote, quote_with_bid, run_child};

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
    // Futures made on another thread while shutdown runs.
    let racing: Vec<Promise<()>> = (0..200).map(|_| Promise::new().unwrap()).collect();
    let racer = thread::spawn(move || racing.into_iter().map(IntoFuture::into_future).collect::<Vec<_>>());

    system::shutdown().unwrap();

    assert_eq!(waker.total(), 2);
    // The streams end: the buffered event first.
    assert!(matches!(poll_stream(&mut quotes, &stream_waker), Poll::Ready(Some(_))));
    assert!(matches!(poll_stream(&mut quotes, &stream_waker), Poll::Ready(None)));
    assert!(matches!(poll_stream(&mut waiting, &stream_waker), Poll::Ready(None)));
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
