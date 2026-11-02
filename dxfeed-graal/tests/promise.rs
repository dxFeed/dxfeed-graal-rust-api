//! The promises of last events over `LOCAL_HUB` endpoints: no network needed.

mod support;

use std::sync::mpsc;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use dxfeed_graal::event::{BaseEvent, Candle, Quote, TimeAndSale};
use dxfeed_graal::{
    AnyOf, DXEndpoint, Error, ErrorManager, Executor, JavaExceptionClass, Promise, Promises, Role, live_events,
    live_handles,
};
use support::{Hub, candle, candle_symbol, delivered, quote, same_events, time_and_sale};

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

/// The promises of several symbols are released with the list the SDK returns them in.
#[test]
fn last_events_promises_release_their_handles() {
    let _serial = serial();
    let start = live_handles();
    let hub = Hub::new();
    let with_hub = live_handles();
    let promises = hub.feed.get_last_events_promises::<Quote>(["AAPL", "IBM"]).unwrap();
    assert_eq!(live_handles(), with_hub + 2);
    drop(promises);
    assert_eq!(live_handles(), with_hub);
    drop(hub);
    assert_eq!(live_handles(), start);
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

/// A promise created in Rust completes with an event through the typed `complete`.
#[test]
fn a_new_promise_completes_with_an_event() {
    let _serial = serial();
    let promise = Promise::<Quote>::new().unwrap();
    assert!(!promise.is_done().unwrap());
    assert!(promise.get_result().unwrap().is_none());
    promise.complete(&quote("AAPL")).unwrap();
    assert!(promise.is_done().unwrap());
    // The SDK keeps only the sequence of `time_millis_sequence` (its millis come from the times), so the quote is
    // compared field by field.
    let result = promise.get_result().unwrap().unwrap();
    let expected = quote("AAPL");
    assert_eq!(result.symbol(), "AAPL");
    assert_eq!(
        (result.bid_price, result.ask_price, result.bid_size, result.ask_size),
        (
            expected.bid_price,
            expected.ask_price,
            expected.bid_size,
            expected.ask_size
        )
    );
}

#[test]
fn a_new_promise_completes_with_events_in_order() {
    let _serial = serial();
    let promise = Promise::<Vec<TimeAndSale>>::new().unwrap();
    let mut second = time_and_sale("AAPL");
    second.price = 191.0;
    promise.complete(&[time_and_sale("AAPL"), second.clone()]).unwrap();
    let result = promise.wait(LONG).unwrap();
    assert_eq!(result.len(), 2);
    assert!(same_events(&result[0], &time_and_sale("AAPL")), "{result:?}");
    assert!(same_events(&result[1], &second), "{result:?}");
}

#[test]
fn a_new_unit_promise_completes() {
    let _serial = serial();
    let promise = Promise::<()>::new().unwrap();
    assert!(!promise.is_done().unwrap());
    promise.complete().unwrap();
    assert!(promise.is_done().unwrap());
    assert!(promise.has_result().unwrap());
    promise.wait(LONG).unwrap();
}

#[test]
fn complete_exceptionally_fails_the_promise_with_the_class_and_message() {
    let _serial = serial();
    let promise = Promise::<Quote>::new().unwrap();
    promise
        .complete_exceptionally(JavaExceptionClass::IllegalStateException, "no quote")
        .unwrap();
    assert!(promise.is_done().unwrap());
    assert!(promise.has_exception().unwrap());
    assert!(!promise.is_cancelled().unwrap());
    let exception = promise.get_exception().unwrap().unwrap();
    assert_eq!(exception.get_class_name(), "java.lang.IllegalStateException");
    assert_eq!(exception.get_message(), "no quote");
    // As in Java, the wait fails with a PromiseException caused by the exception.
    match promise.wait(LONG) {
        Err(Error::Java(exception)) => {
            assert_eq!(exception.get_class_name(), "com.dxfeed.promise.PromiseException");
            let cause = exception.get_cause().unwrap();
            assert_eq!(cause.get_class_name(), "java.lang.IllegalStateException");
            assert_eq!(cause.get_message(), "no quote");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn every_exception_class_keeps_its_name() {
    let _serial = serial();
    for class in [
        JavaExceptionClass::RuntimeException,
        JavaExceptionClass::IllegalArgumentException,
        JavaExceptionClass::IllegalStateException,
        JavaExceptionClass::UnsupportedOperationException,
        JavaExceptionClass::IOException,
        JavaExceptionClass::CancellationException,
        JavaExceptionClass::TimeoutException,
    ] {
        let promise = Promise::<()>::failed(class, "message").unwrap();
        let exception = promise.get_exception().unwrap().unwrap();
        assert_eq!(exception.get_class_name(), class.get_name());
        assert_eq!(class.to_string(), class.get_name());
    }
}

#[test]
fn failed_is_done_with_the_exception() {
    let _serial = serial();
    let promise = Promise::<Vec<TimeAndSale>>::failed(JavaExceptionClass::IOException, "gone").unwrap();
    assert!(promise.is_done().unwrap());
    assert!(!promise.has_result().unwrap());
    let exception = promise.get_exception().unwrap().unwrap();
    assert_eq!(exception.get_class_name(), "java.io.IOException");
    assert_eq!(exception.get_message(), "gone");
    assert!(matches!(promise.get_result(), Err(Error::Java(_))));
}

#[test]
fn a_message_with_a_nul_byte_is_invalid() {
    let _serial = serial();
    let promise = Promise::<()>::new().unwrap();
    assert!(matches!(
        promise.complete_exceptionally(JavaExceptionClass::RuntimeException, "a\0b"),
        Err(Error::InvalidArgument(_))
    ));
    assert!(!promise.is_done().unwrap());
    assert!(matches!(
        Promise::<()>::failed(JavaExceptionClass::RuntimeException, "a\0b"),
        Err(Error::InvalidArgument(_))
    ));
}

/// The first completion wins; the later ones do nothing.
#[test]
fn the_first_completion_wins() {
    let _serial = serial();
    let promise = Promise::<Quote>::new().unwrap();
    promise.complete(&quote("AAPL")).unwrap();
    promise.complete(&quote("IBM")).unwrap();
    promise
        .complete_exceptionally(JavaExceptionClass::RuntimeException, "late")
        .unwrap();
    promise.cancel().unwrap();
    assert_eq!(promise.get_result().unwrap().unwrap().symbol(), "AAPL");
    assert!(!promise.has_exception().unwrap());
}

/// The handlers of a new promise run on the completing thread before `complete` returns.
#[test]
fn when_done_runs_on_the_completing_thread() {
    let _serial = serial();
    let promise = Promise::<Quote>::new().unwrap();
    let (sender, receiver) = mpsc::channel();
    promise
        .when_done(move |done| {
            let symbol = done.get_result().unwrap().map(|quote| quote.symbol().into_owned());
            sender.send((thread::current().id(), symbol)).unwrap();
        })
        .unwrap();
    assert!(receiver.try_recv().is_err());
    promise.complete(&quote("AAPL")).unwrap();
    let (thread, symbol) = receiver.try_recv().unwrap();
    assert_eq!(thread, thread::current().id());
    assert_eq!(symbol.as_deref(), Some("AAPL"));
}

#[test]
fn new_promises_combine_with_all_of() {
    let _serial = serial();
    let first = Promise::<Quote>::new().unwrap();
    let second = Promise::<Quote>::new().unwrap();
    let all = Promises::all_of([&first, &second]).unwrap();
    first.complete(&quote("AAPL")).unwrap();
    assert!(!all.is_done().unwrap());
    second
        .complete_exceptionally(JavaExceptionClass::TimeoutException, "late")
        .unwrap();
    assert!(all.is_done().unwrap());
    all.wait(LONG).unwrap();
}

#[test]
fn new_promises_release_their_handles() {
    let _serial = serial();
    let before = live_handles();
    {
        let promise = Promise::<Quote>::new().unwrap();
        promise.when_done(|_| {}).unwrap();
        promise.complete(&quote("AAPL")).unwrap();
        let _failed = Promise::<()>::failed(JavaExceptionClass::RuntimeException, "x").unwrap();
    }
    assert_eq!(live_handles(), before);
}

/// The calls that read a done promise give the exception itself; a positive wait gives the `PromiseException` of Java.
#[test]
fn reads_of_a_failed_promise_give_the_exception() {
    let _serial = serial();
    let promise = Promise::<Quote>::failed(JavaExceptionClass::UnsupportedOperationException, "no").unwrap();
    let class = |result: dxfeed_graal::Result<Quote>| match result {
        Err(Error::Java(exception)) => exception.get_class_name().to_owned(),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        class(promise.wait(Duration::ZERO)),
        "java.lang.UnsupportedOperationException"
    );
    assert_eq!(class(promise.await_result()), "java.lang.UnsupportedOperationException");
    assert_eq!(class(promise.wait(LONG)), "com.dxfeed.promise.PromiseException");
}

/// A handler added to a done promise runs at once on the calling thread.
#[test]
fn when_done_of_a_failed_promise_runs_at_once() {
    let _serial = serial();
    let promise = Promise::<()>::failed(JavaExceptionClass::RuntimeException, "done").unwrap();
    let (sender, receiver) = mpsc::channel();
    promise
        .when_done(move |done| {
            sender
                .send((thread::current().id(), done.has_exception().unwrap()))
                .unwrap()
        })
        .unwrap();
    assert_eq!(receiver.try_recv().unwrap(), (thread::current().id(), true));
}

/// A completion after a failure or a cancellation is ignored, and the handler runs once.
#[test]
fn a_late_completion_is_ignored() {
    let _serial = serial();
    for cancel in [false, true] {
        let promise = Promise::<Quote>::new().unwrap();
        let (sender, receiver) = mpsc::channel();
        promise.when_done(move |_| sender.send(()).unwrap()).unwrap();
        if cancel {
            promise.cancel().unwrap();
        } else {
            promise
                .complete_exceptionally(JavaExceptionClass::RuntimeException, "first")
                .unwrap();
        }
        promise.complete(&quote("AAPL")).unwrap();
        assert!(!promise.has_result().unwrap());
        assert_eq!(promise.is_cancelled().unwrap(), cancel);
        assert_eq!(receiver.try_iter().count(), 1);
    }
}

/// Completing the promise of `all_of` cancels its pending promises.
#[test]
fn completing_all_of_cancels_its_new_promises() {
    let _serial = serial();
    let first = Promise::<Quote>::new().unwrap();
    let second = Promise::<Quote>::new().unwrap();
    first.complete(&quote("AAPL")).unwrap();
    let all = Promises::all_of([&first, &second]).unwrap();
    all.complete().unwrap();
    assert!(second.is_cancelled().unwrap());
    assert!(first.has_result().unwrap());
}
