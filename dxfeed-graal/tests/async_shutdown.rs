//! Shutdown and the futures of promises. The scenario runs in a child process ([`support::run_child`]), because
//! shutdown is process-wide.

mod support;

use std::future::IntoFuture;
use std::task::Poll;
use std::thread;
use std::time::Duration;

use dxfeed_graal::event::Quote;
use dxfeed_graal::{Error, Promise, Promises, system};
use support::{CountingWaker, Hub, block_on_with, is_child, poll_with, run_child};

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
    // Futures made on another thread while shutdown runs.
    let racing: Vec<Promise<()>> = (0..200).map(|_| Promise::new().unwrap()).collect();
    let racer = thread::spawn(move || racing.into_iter().map(IntoFuture::into_future).collect::<Vec<_>>());

    system::shutdown().unwrap();

    assert_eq!(waker.total(), 2);
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
