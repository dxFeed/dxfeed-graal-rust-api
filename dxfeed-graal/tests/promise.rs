//! The promises of last events over `LOCAL_HUB` endpoints: no network needed.

mod support;

use std::sync::mpsc;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use dxfeed_graal::event::{BaseEvent, Candle, Quote};
use dxfeed_graal::{
    AnyOf, DXEndpoint, Error, ErrorManager, Executor, Promise, Promises, Role, live_events, live_handles,
};
use support::{Hub, candle, candle_symbol, delivered, quote, same_events};

/// Runs the tests of this file one by one, so that each can count the live handles and events.
fn serial() -> MutexGuard<'static, ()> {
    static SERIAL: Mutex<()> = Mutex::new(());
    SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
}

const LONG: Duration = Duration::from_secs(10);

#[test]
fn promise_is_send_and_sync() {
    fn check<T: Send + Sync>() {}
    check::<Promise<Quote>>();
    check::<Promise<()>>();
    check::<AnyOf<Quote>>();
}

/// A promise of a cached event is done at once.
#[test]
fn cached_event_completes_the_promise() {
    let _serial = serial();
    let hub = Hub::new();
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    subscription.add_symbols(["AAPL"]).unwrap();
    hub.publish_and_process(&[quote("AAPL").into()]);

    let promise = hub.feed.get_last_event_promise::<Quote>("AAPL").unwrap();
    assert!(promise.is_done().unwrap());
    let expected = delivered(quote("AAPL").into());
    assert!(same_events(&promise.wait(LONG).unwrap().into(), &expected));
    assert!(same_events(&promise.get_result().unwrap().unwrap().into(), &expected));
    // The timeout is clamped, so the longest one waits as well.
    assert!(same_events(&promise.wait(Duration::MAX).unwrap().into(), &expected));
    assert!(!promise.is_cancelled().unwrap());
    drop(promise);
    assert_eq!(live_events(), 0);
}

/// A promise made before the data subscribes by itself and completes when the event is published.
#[test]
fn published_event_completes_a_pending_promise() {
    let _serial = serial();
    let hub = Hub::new();
    let c = &candle_symbol("AAPL");
    let quotes = hub.feed.get_last_event_promise::<Quote>("AAPL").unwrap();
    let candles = hub.feed.get_last_event_promise::<Candle>(c).unwrap();
    assert!(!quotes.is_done().unwrap());
    assert!(quotes.get_result().unwrap().is_none());

    hub.publish_and_process(&[quote("AAPL").into(), candle(c).into()]);
    assert!(same_events(
        &quotes.wait(LONG).unwrap().into(),
        &delivered(quote("AAPL").into())
    ));
    assert!(same_events(
        &candles.wait(LONG).unwrap().into(),
        &delivered(candle(c).into())
    ));
}

/// A zero timeout only checks a pending promise; a positive one cancels it when it expires.
#[test]
fn timeouts() {
    let _serial = serial();
    let hub = Hub::new();
    let promise = hub.feed.get_last_event_promise::<Quote>("AAPL").unwrap();
    assert!(matches!(promise.wait(Duration::ZERO), Err(Error::Timeout)));
    assert!(!promise.is_done().unwrap());

    let start = Instant::now();
    assert!(matches!(promise.wait(Duration::from_millis(200)), Err(Error::Timeout)));
    assert!(start.elapsed() >= Duration::from_millis(200));
    assert!(promise.is_cancelled().unwrap());
    assert!(matches!(promise.get_result(), Err(Error::Java(_))));
    assert!(matches!(promise.wait(LONG), Err(Error::Java(_))));

    // A timeout under a millisecond is rounded up, so it waits and cancels rather than only checking.
    let promise = hub.feed.get_last_event_promise::<Quote>("IBM").unwrap();
    assert!(matches!(promise.wait(Duration::from_micros(500)), Err(Error::Timeout)));
    assert!(promise.is_cancelled().unwrap());
}

/// Cancelling from another thread wakes a waiter long before its timeout.
#[test]
fn cancel_wakes_a_waiter() {
    let _serial = serial();
    let hub = Hub::new();
    let promise = hub.feed.get_last_event_promise::<Quote>("AAPL").unwrap();
    let start = Instant::now();
    thread::scope(|s| {
        s.spawn(|| {
            thread::sleep(Duration::from_millis(100));
            promise.cancel().unwrap();
        });
        let result = promise.wait(LONG);
        assert!(matches!(result, Err(Error::Java(_))), "{result:?}");
    });
    assert!(start.elapsed() < LONG / 2);
    assert!(promise.is_cancelled().unwrap());
}

/// Closing the endpoint does not complete its pending promises, despite the SDK docs: only a timeout ends the wait.
#[test]
fn closing_the_endpoint_leaves_a_promise_pending() {
    let _serial = serial();
    let hub = Hub::with_default_executor();
    let promise = hub.feed.get_last_event_promise::<Quote>("AAPL").unwrap();
    hub.endpoint.close().unwrap();
    assert!(!promise.is_done().unwrap());
    assert!(matches!(promise.wait(Duration::from_millis(100)), Err(Error::Timeout)));
    assert!(promise.is_cancelled().unwrap());
}

#[test]
fn stream_feed_promise_completes_exceptionally() {
    let _serial = serial();
    let endpoint = DXEndpoint::new_builder().with_role(Role::StreamFeed).build().unwrap();
    let promise = endpoint.get_feed().get_last_event_promise::<Quote>("AAPL").unwrap();
    let result = promise.wait(LONG);
    assert!(matches!(result, Err(Error::Java(_))), "{result:?}");
    assert!(!promise.is_cancelled().unwrap());
}

/// A listener may query and cancel a promise, but not block on it.
#[test]
fn listener_cannot_block_on_a_promise() {
    let _serial = serial();
    let hub = Hub::new();
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    let promise = hub.feed.get_last_event_promise::<Quote>("IBM").unwrap();
    let (sender, receiver) = mpsc::channel();
    subscription
        .add_event_listener(move |quotes| {
            if quotes.count() > 0 {
                let blocking = promise.wait(LONG);
                let zero = promise.wait(Duration::ZERO);
                let done = promise.is_done();
                let cancel = promise.cancel();
                let _ = sender.send((blocking, zero, done, cancel, promise.is_cancelled()));
            }
        })
        .unwrap();
    subscription.add_symbols(["AAPL"]).unwrap();
    hub.publish_and_process(&[quote("AAPL").into()]);
    let (blocking, zero, done, cancel, cancelled) = receiver.try_recv().unwrap();
    assert!(matches!(blocking, Err(Error::InCallback)), "{blocking:?}");
    assert!(matches!(zero, Err(Error::Timeout)), "{zero:?}");
    assert!(!done.unwrap());
    cancel.unwrap();
    assert!(cancelled.unwrap());
}

/// The promise keeps the endpoint open while it lives, and dropping it releases its handle.
#[test]
fn dropping_a_promise_releases_it() {
    let _serial = serial();
    let start = live_handles();
    let hub = Hub::new();
    let promise = hub.feed.get_last_event_promise::<Quote>("AAPL").unwrap();
    drop(hub);
    assert!(!promise.is_cancelled().unwrap());
    // The promise, and the endpoint, feed and publisher of the feed it holds.
    assert_eq!(live_handles(), start + 4);
    drop(promise);
    assert_eq!(live_handles(), start);
    assert_eq!(live_events(), 0);
}

/// A handler of a done promise runs at once on the calling thread; one of a pending promise runs once it completes.
#[test]
fn when_done_runs_once_when_the_promise_is_done() {
    let _serial = serial();
    let start = live_handles();
    let hub = Hub::new();
    // A subscription keeps the last event, which completes a new promise at once.
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    subscription.add_symbols(["AAPL"]).unwrap();
    hub.publish_and_process(&[quote("AAPL").into()]);
    let done = hub.feed.get_last_event_promise::<Quote>("AAPL").unwrap();
    assert!(done.is_done().unwrap());
    let (sender, received) = mpsc::channel();
    let me = thread::current().id();
    done.when_done(move |promise| {
        let _ = sender.send((
            thread::current().id(),
            promise.get_result().unwrap().map(|quote| quote.symbol().into_owned()),
        ));
    })
    .unwrap();
    assert_eq!(received.try_recv().unwrap(), (me, Some("AAPL".to_owned())));

    let pending = hub.feed.get_last_event_promise::<Quote>("IBM").unwrap();
    let (sender, received) = mpsc::channel();
    pending
        .when_done(move |promise| {
            let _ = sender.send(promise.wait(LONG).map(|quote| quote.symbol().into_owned()));
        })
        .unwrap();
    assert!(received.try_recv().is_err(), "a pending promise runs no handler");
    hub.publish_and_process(&[quote("IBM").into()]);
    assert!(
        matches!(received.try_recv().unwrap(), Err(Error::InCallback)),
        "a handler cannot block"
    );
    assert!(received.try_recv().is_err(), "the handler runs once");
    drop((done, pending, subscription, hub));
    assert_eq!(live_handles(), start, "the handlers free their promises");
}

/// A handler on an executor runs on a thread of it; a panic of a handler goes to the error manager.
#[test]
fn when_done_async_runs_on_the_executor_and_reports_panics() {
    let _serial = serial();
    let start = live_handles();
    let hub = Hub::new();
    let promise = hub.feed.get_last_event_promise::<Quote>("AAPL").unwrap();
    let executor = Executor::new_fixed_thread_pool(1, "promise-handlers").unwrap();
    let (sender, received) = mpsc::channel();
    let me = thread::current().id();
    promise
        .when_done_async(
            move |_| {
                let _ = sender.send(thread::current().id());
            },
            &executor,
        )
        .unwrap();
    ErrorManager::clear_errors();
    promise.when_done(|_| panic!("handler failure")).unwrap();
    hub.publish_and_process(&[quote("AAPL").into()]);
    assert_ne!(received.recv_timeout(LONG).unwrap(), me);
    assert!(
        ErrorManager::get_errors()
            .iter()
            .any(|error| error.to_string().contains("handler failure")),
        "{:?}",
        ErrorManager::get_errors()
    );
    drop((promise, executor, hub));
    // The executor thread releases the handle of the handler after it has sent its message.
    let deadline = Instant::now() + LONG;
    while live_handles() != start && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(live_handles(), start);
}

/// Cancelling an `AnyOf` makes it done at once, before the executor runs the handlers.
#[test]
fn cancelling_any_of_makes_it_done() {
    let _serial = serial();
    let hub = Hub::new();
    let promises = ["AAPL", "IBM"].map(|symbol| hub.feed.get_last_event_promise::<Quote>(symbol).unwrap());
    let any = Promises::any_of(promises.into()).unwrap();
    any.cancel().unwrap();
    assert!(any.is_done());
    assert!(matches!(any.get_result(), Err(Error::Java(_))));
    assert!(any.get_promises().iter().all(|promise| promise.is_cancelled().unwrap()));
    hub.executor.process_all_pending().unwrap();
    assert_eq!(any.get_done_index(), Some(0));
}

/// Waiting without a timeout, and the result and exception flags.
#[test]
fn await_result_and_the_exception() {
    let _serial = serial();
    let hub = Hub::new();
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    subscription.add_symbols(["AAPL"]).unwrap();
    hub.publish_and_process(&[quote("AAPL").into()]);
    let promise = hub.feed.get_last_event_promise::<Quote>("AAPL").unwrap();
    assert_eq!(promise.await_result().unwrap().symbol(), "AAPL");
    assert!(promise.has_result().unwrap());
    assert!(!promise.has_exception().unwrap());
    assert!(promise.get_exception().unwrap().is_none());

    let cancelled = hub.feed.get_last_event_promise::<Quote>("IBM").unwrap();
    cancelled.cancel().unwrap();
    assert!(matches!(cancelled.await_result(), Err(Error::Java(_))));
    assert!(!cancelled.has_result().unwrap());
    assert!(cancelled.has_exception().unwrap());
    let exception = cancelled.get_exception().unwrap().unwrap();
    assert!(exception.to_string().contains("CancellationException"), "{exception}");
}

/// `all_of` completes with the last of its promises; `any_of` with the first, cancelling the others.
#[test]
fn all_of_and_any_of() {
    let _serial = serial();
    let start = live_handles();
    let hub = Hub::new();
    let symbols = ["AAPL", "IBM", "MSFT"];
    let requested: Vec<Promise<Quote>> = symbols
        .iter()
        .map(|symbol| hub.feed.get_last_event_promise::<Quote>(symbol).unwrap())
        .collect();
    let all = Promises::all_of(&requested).unwrap();
    hub.publish_and_process(&[quote("AAPL").into(), quote("IBM").into()]);
    assert!(!all.is_done().unwrap());
    hub.publish_and_process(&[quote("MSFT").into()]);
    all.wait(Duration::ZERO).unwrap();
    assert!(requested.iter().all(|promise| promise.has_result().unwrap()));
    assert!(
        Promises::all_of(&Vec::<Promise<Quote>>::new())
            .unwrap()
            .is_done()
            .unwrap()
    );

    let promises: Vec<Promise<Quote>> = symbols
        .iter()
        .map(|symbol| {
            hub.feed
                .get_last_event_promise::<Quote>(&format!("{symbol}.X"))
                .unwrap()
        })
        .collect();
    let any = Promises::any_of(promises).unwrap();
    assert!(!any.is_done());
    assert!(matches!(any.wait(Duration::ZERO), Err(Error::Timeout)));
    hub.publish_and_process(&[quote("IBM.X").into()]);
    let (index, first) = any.wait(LONG).unwrap();
    assert_eq!(index, 1);
    assert_eq!(first.symbol(), "IBM.X");
    assert!(any.get_promises()[0].is_cancelled().unwrap());
    assert!(any.get_promises()[2].is_cancelled().unwrap());
    assert!(matches!(
        Promises::any_of(Vec::<Promise<Quote>>::new()),
        Err(Error::InvalidArgument(_))
    ));

    drop((requested, all, any, hub));
    assert_eq!(live_handles(), start);
}
