//! The promises of last events over `LOCAL_HUB` endpoints: no network needed.

mod support;

use std::sync::mpsc;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use dxfeed_graal::event::{Candle, Quote};
use dxfeed_graal::{DXEndpoint, Error, Promise, Role, live_events, live_handles};
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
