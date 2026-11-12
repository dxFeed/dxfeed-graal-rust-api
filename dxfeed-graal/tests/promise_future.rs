//! The futures of promises: of feed requests over `LOCAL_HUB` endpoints and of promises the application completes.

mod support;

use std::fmt::Debug;
use std::future::IntoFuture;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Poll, Wake};
use std::thread;
use std::time::Duration;

use dxfeed_graal::event::{Candle, Quote};
use dxfeed_graal::{AnyOfFuture, Error, JavaExceptionClass, Promise, PromiseFuture, Promises, live_handles};
use support::{CountingWaker, Hub, block_on, block_on_with, delivered, poll_with, quote, same_events};

/// Runs the tests of this file one by one, so that each can count the live handles.
fn serial() -> MutexGuard<'static, ()> {
    static SERIAL: Mutex<()> = Mutex::new(());
    SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
}

const LONG: Duration = Duration::from_secs(10);

/// The class name of the Java exception of `result`; panics on anything else.
fn exception_class<T: Debug>(result: &Result<T, Error>) -> &str {
    match result {
        Err(Error::Java(exception)) => exception.get_class_name(),
        other => panic!("not a Java exception: {other:?}"),
    }
}

#[test]
fn futures_are_send_sync_unpin_and_static() {
    fn check<T: Send + Sync + Unpin + 'static>() {}
    check::<PromiseFuture<Quote>>();
    check::<PromiseFuture<()>>();
    check::<PromiseFuture<Vec<Candle>>>();
    check::<AnyOfFuture<Quote>>();
}

/// A done promise is ready at the first poll: its handler ran while the future was made.
#[test]
fn done_promise_is_ready_at_once() {
    let _serial = serial();
    let promise = Promise::<()>::new().unwrap();
    promise.complete().unwrap();
    let waker = CountingWaker::new();
    let mut future = promise.into_future();
    assert!(matches!(poll_with(&mut future, &waker), Poll::Ready(Ok(()))));
    assert_eq!(waker.total(), 0);
}

/// A promise completed on another thread wakes the task once.
#[test]
fn promise_completed_on_another_thread() {
    let _serial = serial();
    let promise = Promise::<()>::new().unwrap();
    let mut future = Promises::all_of([&promise]).unwrap().into_future();
    let waker = CountingWaker::new();
    assert!(poll_with(&mut future, &waker).is_pending());
    assert!(!future.is_done().unwrap());
    thread::spawn(move || promise.complete().unwrap()).join().unwrap();
    assert!(matches!(block_on_with(future, &waker, LONG), Some(Ok(()))));
    assert_eq!((waker.wakes(), waker.wakes_by_ref()), (1, 0));
}

/// The waker of the last poll is the one woken.
#[test]
fn the_last_waker_is_woken() {
    let _serial = serial();
    let promise = Promise::<()>::new().unwrap();
    let mut future = Promises::all_of([&promise]).unwrap().into_future();
    let (first, last) = (CountingWaker::new(), CountingWaker::new());
    assert!(poll_with(&mut future, &first).is_pending());
    assert!(poll_with(&mut future, &last).is_pending());
    promise.complete().unwrap();
    assert_eq!((first.total(), last.total()), (0, 1));
    assert!(matches!(poll_with(&mut future, &last), Poll::Ready(Ok(()))));
}

/// A promise completed exceptionally gives its exception.
#[test]
fn failed_promise_gives_its_exception() {
    let _serial = serial();
    let promise = Promise::<Quote>::failed(JavaExceptionClass::IllegalStateException, "no quote").unwrap();
    let result = block_on(promise);
    assert_eq!(exception_class(&result), "java.lang.IllegalStateException");
}

/// A feed request on a manual executor is ready once the executor runs the handler.
#[test]
fn feed_request_is_ready_after_the_executor_runs() {
    let _serial = serial();
    let hub = Hub::new();
    let waker = CountingWaker::new();
    let mut future = hub.feed.get_last_event_promise::<Quote>("AAPL").unwrap().into_future();
    assert!(poll_with(&mut future, &waker).is_pending());
    hub.publisher.publish_events(&[quote("AAPL").into()]).unwrap();
    assert!(poll_with(&mut future, &waker).is_pending());
    assert_eq!(waker.total(), 0);

    hub.executor.process_all_pending().unwrap();
    assert_eq!(waker.total(), 1);
    match poll_with(&mut future, &waker) {
        Poll::Ready(Ok(received)) => assert!(same_events(&received.into(), &delivered(quote("AAPL").into()))),
        other => panic!("{other:?}"),
    }
}

/// Cancelling the future cancels the promise and makes the future ready without the executor.
#[test]
fn cancel_makes_the_future_ready_at_once() {
    let _serial = serial();
    let hub = Hub::new();
    let waker = CountingWaker::new();
    let mut future = hub.feed.get_last_event_promise::<Quote>("AAPL").unwrap().into_future();
    assert!(poll_with(&mut future, &waker).is_pending());
    future.cancel().unwrap();
    assert_eq!(waker.total(), 1);
    assert!(future.is_cancelled().unwrap());
    match poll_with(&mut future, &waker) {
        Poll::Ready(result) => assert_eq!(exception_class(&result), "java.util.concurrent.CancellationException"),
        Poll::Pending => panic!("pending after cancel"),
    }
    // The handler of the request runs later and finds the future done.
    hub.executor.process_all_pending().unwrap();
}

/// Dropping a pending future cancels nothing; the handler of its promise runs harmlessly later and frees its handle.
#[test]
fn dropping_a_future_does_not_cancel() {
    let _serial = serial();
    let baseline = live_handles();
    let promise = Promise::<()>::new().unwrap();
    // `all_of` cancels its promises when it is cancelled, so the promise shows whether the future cancelled it.
    let mut future = Promises::all_of([&promise]).unwrap().into_future();
    assert!(poll_with(&mut future, &CountingWaker::new()).is_pending());
    drop(future);
    assert!(!promise.is_done().unwrap());
    promise.complete().unwrap();
    drop(promise);
    assert_eq!(live_handles(), baseline);
}

/// Dropping a future of a promise that never completes releases the waker.
#[test]
fn dropping_a_future_releases_the_waker() {
    let _serial = serial();
    let promise = Promise::<()>::new().unwrap();
    let mut future = Promises::all_of([&promise]).unwrap().into_future();
    let waker = CountingWaker::new();
    let baseline = Arc::strong_count(&waker);
    assert!(poll_with(&mut future, &waker).is_pending());
    assert_eq!(Arc::strong_count(&waker), baseline + 1);
    drop(future);
    assert_eq!(Arc::strong_count(&waker), baseline);
    promise.complete().unwrap();
}

#[test]
#[should_panic(expected = "PromiseFuture polled after completion")]
fn polling_a_promise_future_after_completion_panics() {
    let _serial = serial();
    let promise = Promise::<()>::new().unwrap();
    promise.complete().unwrap();
    let (mut future, waker) = (promise.into_future(), CountingWaker::new());
    assert!(poll_with(&mut future, &waker).is_ready());
    let _ = poll_with(&mut future, &waker);
}

/// The first of two feed requests done wins; the other is cancelled.
#[test]
fn any_of_gives_the_first_done() {
    let _serial = serial();
    let hub = Hub::new();
    let aapl = hub.feed.get_last_event_promise::<Quote>("AAPL").unwrap();
    let (sender, cancelled) = mpsc::channel();
    aapl.when_done(move |promise| sender.send(promise.is_cancelled().unwrap()).unwrap())
        .unwrap();
    let msft = hub.feed.get_last_event_promise::<Quote>("MSFT").unwrap();
    let mut future = Promises::any_of(vec![aapl, msft]).unwrap().into_future();
    let waker = CountingWaker::new();
    assert!(poll_with(&mut future, &waker).is_pending());

    hub.publish_and_process(&[quote("MSFT").into()]);
    assert_eq!(waker.total(), 1);
    assert!(future.is_done());
    match poll_with(&mut future, &waker) {
        Poll::Ready(Ok((index, received))) => {
            assert_eq!(index, 1);
            assert!(same_events(&received.into(), &delivered(quote("MSFT").into())));
        }
        other => panic!("{other:?}"),
    }
    hub.executor.process_all_pending().unwrap();
    assert!(cancelled.recv_timeout(LONG).unwrap());
}

/// When the first promise done failed, the future gives its exception.
#[test]
fn any_of_gives_the_exception_of_the_first_done() {
    let _serial = serial();
    let failed = Promise::<Quote>::failed(JavaExceptionClass::IllegalStateException, "no quote").unwrap();
    let pending = Promise::<Quote>::new().unwrap();
    let result = block_on(Promises::any_of(vec![failed, pending]).unwrap());
    assert_eq!(exception_class(&result), "java.lang.IllegalStateException");
}

/// Cancelling the future of `any_of` makes it ready without the executor.
#[test]
fn any_of_cancel_makes_the_future_ready_at_once() {
    let _serial = serial();
    let hub = Hub::new();
    let promises = ["AAPL", "MSFT"].map(|symbol| hub.feed.get_last_event_promise::<Quote>(symbol).unwrap());
    let mut future = Promises::any_of(Vec::from(promises)).unwrap().into_future();
    let waker = CountingWaker::new();
    assert!(poll_with(&mut future, &waker).is_pending());
    future.cancel().unwrap();
    assert_eq!(waker.total(), 1);
    match poll_with(&mut future, &waker) {
        Poll::Ready(result) => assert_eq!(exception_class(&result), "java.util.concurrent.CancellationException"),
        Poll::Pending => panic!("pending after cancel"),
    }
    hub.executor.process_all_pending().unwrap();
}

#[test]
#[should_panic(expected = "AnyOfFuture polled after completion")]
fn polling_an_any_of_future_after_completion_panics() {
    let _serial = serial();
    let promise = Promise::<()>::new().unwrap();
    promise.complete().unwrap();
    let (mut future, waker) = (
        Promises::any_of(vec![promise]).unwrap().into_future(),
        CountingWaker::new(),
    );
    assert!(poll_with(&mut future, &waker).is_ready());
    let _ = poll_with(&mut future, &waker);
}

/// A waker that formats the future when woken, which takes the lock of its state: a wake under that lock would
/// deadlock.
struct Reentrant<F: Debug + Send> {
    future: Arc<Mutex<F>>,
    woken: Sender<String>,
}

impl<F: Debug + Send> Wake for Reentrant<F> {
    fn wake(self: Arc<Self>) {
        let text = format!("{:?}", self.future.lock().unwrap());
        let _ = self.woken.send(text);
    }
}

/// Polls `future` with a [`Reentrant`] waker, completes `promise` on another thread and returns what the waker saw.
fn wake_outside_the_lock<F>(future: F, promise: Promise<()>) -> String
where
    F: std::future::Future + Debug + Send + Unpin + 'static,
{
    let (woken, seen) = mpsc::channel();
    let future = Arc::new(Mutex::new(future));
    let waker = Arc::new(Reentrant {
        future: Arc::clone(&future),
        woken,
    })
    .into();
    let mut context = std::task::Context::from_waker(&waker);
    assert!(
        std::pin::Pin::new(&mut *future.lock().unwrap())
            .poll(&mut context)
            .is_pending()
    );
    thread::spawn(move || promise.complete().unwrap());
    seen.recv_timeout(LONG).expect("the waker deadlocked or was not woken")
}

#[test]
fn promise_future_wakes_outside_its_lock() {
    let _serial = serial();
    let promise = Promise::<()>::new().unwrap();
    let future = Promises::all_of([&promise]).unwrap().into_future();
    let seen = wake_outside_the_lock(future, promise);
    assert!(seen.contains("ready: true"), "{seen}");
}

#[test]
fn any_of_future_wakes_outside_its_lock() {
    let _serial = serial();
    let promise = Promise::<()>::new().unwrap();
    let future = Promises::any_of(vec![Promises::all_of([&promise]).unwrap()])
        .unwrap()
        .into_future();
    let seen = wake_outside_the_lock(future, promise);
    assert!(seen.contains("done_index: Some(0)"), "{seen}");
}

/// Completions racing the first poll lose no wake-up.
#[test]
fn completion_racing_the_poll_is_not_lost() {
    let _serial = serial();
    for i in 0..200 {
        let promise = Promise::<()>::new().unwrap();
        let all = Promises::all_of([&promise]).unwrap();
        let completer = thread::spawn(move || promise.complete().unwrap());
        if i % 2 == 0 {
            assert!(matches!(block_on(all), Ok(())));
        } else {
            assert!(matches!(block_on(Promises::any_of(vec![all]).unwrap()), Ok((0, ()))));
        }
        completer.join().unwrap();
    }
}
