//! Characterization of the SDK behavior that the futures and the event streams rely on, over `LOCAL_HUB` endpoints,
//! and of the test executor in `support`.

mod support;

use std::future::{Future, ready};
use std::pin::Pin;
use std::sync::mpsc::{self, Sender};
use std::task::{Context, Poll};
use std::thread;
use std::time::Duration;

use dxfeed_graal::event::Quote;
use dxfeed_graal::{Error, ObservableSubscriptionChangeListener, Promise, Symbol, system};
use support::{CountingWaker, Hub, block_on, block_on_with, collect, is_child, poll_with, quote, run_child};

#[test]
fn block_on_returns_a_ready_value() {
    assert_eq!(block_on(ready(7)), 7);
}

/// A future that is never ready and never wakes.
struct Never;

impl Future for Never {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<()> {
        Poll::Pending
    }
}

#[test]
fn block_on_gives_up_at_the_timeout() {
    assert!(block_on_with(Never, &CountingWaker::new(), Duration::from_millis(50)).is_none());
}

/// A future that wakes its waker by reference once, then by value once, then is ready.
struct WakeTwice(usize);

impl Future for WakeTwice {
    type Output = usize;

    #[allow(clippy::waker_clone_wake, reason = "the test counts a wake by value")]
    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<usize> {
        self.0 += 1;
        match self.0 {
            1 => context.waker().wake_by_ref(),
            2 => context.waker().clone().wake(),
            _ => return Poll::Ready(self.0),
        }
        Poll::Pending
    }
}

#[test]
fn counting_waker_counts_wake_and_wake_by_ref() {
    let waker = CountingWaker::new();
    let mut future = WakeTwice(0);
    assert!(poll_with(&mut future, &waker).is_pending());
    assert_eq!((waker.wakes(), waker.wakes_by_ref()), (0, 1));
    assert_eq!(block_on_with(future, &waker, Duration::from_secs(10)), Some(3));
    assert_eq!((waker.wakes(), waker.wakes_by_ref()), (1, 1));
    assert_eq!(waker.total(), 2);
}

/// Sends to the channel when the subscription is closed.
struct OnClosed(Sender<()>);

impl ObservableSubscriptionChangeListener for OnClosed {
    fn symbols_added(&mut self, _: &[Symbol]) {}

    fn subscription_closed(&mut self) {
        let _ = self.0.send(());
    }
}

/// How a probe closes the endpoint.
#[derive(Clone, Copy, Debug)]
enum Close {
    Close,
    AwaitTermination,
}

/// Closes the endpoint of a subscription with a change listener; returns whether the listener was told that the
/// subscription is closed, and whether the subscription says it is closed.
fn close_endpoint(hub: &Hub, close: Close, manual: bool) -> (bool, bool) {
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    let (sender, closed) = mpsc::channel();
    subscription.add_change_listener(OnClosed(sender)).unwrap();
    subscription.add_symbols(["AAPL"]).unwrap();
    match close {
        Close::Close => hub.endpoint.close().unwrap(),
        Close::AwaitTermination => hub.endpoint.close_and_await_termination().unwrap(),
    }
    if manual {
        let _ = hub.executor.process_all_pending();
    }
    let notified = closed.recv_timeout(Duration::from_secs(1)).is_ok();
    let observed = (notified, subscription.is_closed().unwrap());
    println!("PROBE endpoint {close:?}, manual executor {manual}: (notified, is_closed) = {observed:?}");
    observed
}

/// Closing the endpoint neither closes its subscriptions nor tells their change listeners, so a stream needs the
/// endpoint to end it.
#[test]
fn endpoint_close_leaves_subscriptions_open() {
    let hub = Hub::with_default_executor();
    assert_eq!(close_endpoint(&hub, Close::Close, false), (false, false));
    let hub = Hub::with_default_executor();
    assert_eq!(close_endpoint(&hub, Close::AwaitTermination, false), (false, false));
    assert_eq!(close_endpoint(&Hub::new(), Close::Close, true), (false, false));
}

/// A subscription with symbols refuses a new event listener, so a stream is made before the symbols are added.
#[test]
fn listener_is_refused_after_symbols() {
    let hub = Hub::new();
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    subscription.add_symbols(["AAPL"]).unwrap();
    hub.publish_and_process(&[quote("AAPL").into()]);
    let added = subscription.add_event_listener(|_| {});
    println!("PROBE listener added after symbols: {added:?}");
    assert!(
        matches!(&added, Err(Error::Java(e)) if e.get_class_name() == "java.lang.IllegalStateException"),
        "{added:?}"
    );
    // Without symbols it is accepted, and the symbols added afterwards bring the last event.
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    let events = collect(&subscription);
    subscription.add_symbols(["AAPL"]).unwrap();
    hub.executor.process_all_pending().unwrap();
    assert!(events.try_recv().is_ok());
}

/// Shutdown leaves pending promises pending and runs none of their handlers, so the futures need their own shutdown
/// path.
#[test]
fn shutdown_leaves_pending_promises_pending() {
    let (stdout, _) = run_child("child_shutdown_and_pending_promise_handlers");
    let line = stdout.lines().find(|l| l.starts_with("PROBE")).unwrap_or_default();
    println!("{line}");
    assert!(
        line.contains("handlers run: []; done: request Ok(false), created Ok(false)"),
        "{stdout}"
    );
}

#[test]
#[ignore = "run by shutdown_leaves_pending_promises_pending"]
fn child_shutdown_and_pending_promise_handlers() {
    if !is_child() {
        return;
    }
    let hub = Hub::with_default_executor();
    let request = hub.feed.get_last_event_promise::<Quote>("AAPL").unwrap();
    let created = Promise::<Quote>::new().unwrap();
    let (sender, ran) = mpsc::channel();
    for (name, promise) in [("request", &request), ("created", &created)] {
        let sender = sender.clone();
        promise
            .when_done(move |_| {
                let _ = sender.send(name);
            })
            .unwrap();
    }
    system::shutdown().unwrap();
    thread::sleep(Duration::from_millis(500));
    let ran: Vec<&str> = ran.try_iter().collect();
    println!(
        "PROBE shutdown, handlers run: {ran:?}; done: request {:?}, created {:?}",
        request.is_done(),
        created.is_done()
    );
}
