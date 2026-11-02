//! The tuning of a subscription: aggregation period, events batch limit, executor and controller.

mod support;

use std::sync::{Arc, OnceLock, Weak, mpsc};
use std::thread;
use std::time::Duration;

use dxfeed_graal::event::Quote;
use dxfeed_graal::{
    DXFeedSubscription, Error, Executor, ObservableSubscriptionChangeListener, Symbol, TimePeriod, TimePeriodInfo,
};
use support::{Hub, quote_with_bid};

#[test]
fn the_constants_are_those_of_java() {
    assert_eq!(DXFeedSubscription::AGGREGATION_PERIOD_MAX_VALUE.get_time(), 86_400_000);
    assert_eq!(DXFeedSubscription::OPTIMAL_BATCH_LIMIT, 0);
    assert_eq!(DXFeedSubscription::MAX_BATCH_LIMIT, i32::MAX);
}

#[test]
fn a_fresh_subscription_has_the_defaults() {
    let hub = Hub::new();
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    assert_eq!(subscription.get_aggregation_period().unwrap(), None);
    assert_eq!(
        subscription.get_aggregation_period_info().unwrap(),
        TimePeriodInfo::UNKNOWN
    );
    assert_eq!(
        subscription.get_events_batch_limit().unwrap(),
        DXFeedSubscription::OPTIMAL_BATCH_LIMIT
    );
    assert_eq!(subscription.get_total_dropped_events().unwrap(), 0);
    assert!(subscription.get_executor().unwrap().is_none());
}

#[test]
fn the_aggregation_period_reads_back() {
    let hub = Hub::new();
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    subscription
        .set_aggregation_period(Some(TimePeriod::value_of("1s").unwrap()))
        .unwrap();
    let period = subscription.get_aggregation_period().unwrap().unwrap();
    assert_eq!(period.get_time(), 1000);

    let max = DXFeedSubscription::AGGREGATION_PERIOD_MAX_VALUE;
    subscription.set_aggregation_period(Some(max)).unwrap();
    assert_eq!(subscription.get_aggregation_period().unwrap(), Some(max));
    let too_long = TimePeriod::value_of_millis(max.get_time() + 1);
    match subscription.set_aggregation_period(Some(too_long)) {
        Err(Error::InvalidArgument(message)) => assert_eq!(message, "Aggregation period is too large: P1DT0.001S"),
        other => panic!("expected InvalidArgument, got {other:?}"),
    }
    assert!(matches!(
        subscription.set_aggregation_period(Some(TimePeriod::value_of_millis(-1))),
        Err(Error::Java(_))
    ));
    assert_eq!(subscription.get_aggregation_period().unwrap(), Some(max));

    subscription.set_aggregation_period(None).unwrap();
    assert_eq!(subscription.get_aggregation_period().unwrap(), None);
}

#[test]
fn the_events_batch_limit_must_not_be_negative() {
    let hub = Hub::new();
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    assert!(matches!(
        subscription.set_events_batch_limit(-1),
        Err(Error::InvalidArgument(_))
    ));
    for limit in [
        DXFeedSubscription::OPTIMAL_BATCH_LIMIT,
        5,
        DXFeedSubscription::MAX_BATCH_LIMIT,
    ] {
        subscription.set_events_batch_limit(limit).unwrap();
        assert_eq!(subscription.get_events_batch_limit().unwrap(), limit);
    }
}

#[test]
fn the_executor_reads_back() {
    let hub = Hub::new();
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    let pool = Executor::new_fixed_thread_pool(1, "subscription-pool").unwrap();
    subscription.set_executor(Some(&pool)).unwrap();
    assert!(subscription.get_executor().unwrap().is_some());
    subscription.set_executor(None).unwrap();
    assert!(subscription.get_executor().unwrap().is_none());
}

#[test]
fn the_controller_tunes_and_attaches_the_subscription() {
    let hub = Hub::new();
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    let (sender, received) = mpsc::channel();
    subscription
        .add_event_listener(move |quotes| {
            let _ = sender.send(quotes.count());
        })
        .unwrap();
    subscription.add_symbols(["IBM"]).unwrap();
    let controller = subscription.get_subscription_controller().unwrap();
    controller.set_events_batch_limit(9).unwrap();
    assert_eq!(subscription.get_events_batch_limit().unwrap(), 9);
    controller
        .set_aggregation_period(Some(TimePeriod::value_of_millis(10)))
        .unwrap();
    assert_eq!(
        subscription.get_aggregation_period().unwrap(),
        Some(TimePeriod::value_of_millis(10))
    );
    controller.set_aggregation_period(None).unwrap();

    controller.detach(&hub.feed).unwrap();
    hub.publish_and_process(&[quote_with_bid("IBM", 1.0).into()]);
    assert!(received.try_recv().is_err(), "a detached subscription gets nothing");
    controller.attach(&hub.feed).unwrap();
    hub.publish_and_process(&[quote_with_bid("IBM", 2.0).into()]);
    assert_eq!(received.try_iter().sum::<usize>(), 1);
}

/// Publishes ten quotes 50 ms apart and counts the calls of the listener until the last one arrives.
fn batches(period: Option<TimePeriod>) -> (usize, TimePeriodInfo) {
    let hub = Hub::with_default_executor();
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    subscription.set_aggregation_period(period).unwrap();
    let (sender, received) = mpsc::channel();
    subscription
        .add_event_listener(move |quotes| {
            let last = quotes.map(|quote| quote.bid_price()).fold(f64::NAN, |_, bid| bid);
            let _ = sender.send(last);
        })
        .unwrap();
    subscription.add_symbols(["IBM"]).unwrap();
    for bid in 1..=10 {
        hub.publisher
            .publish_events(&[quote_with_bid("IBM", f64::from(bid)).into()])
            .unwrap();
        thread::sleep(Duration::from_millis(50));
    }
    let mut calls = 0;
    loop {
        let last = received
            .recv_timeout(Duration::from_secs(10))
            .expect("the last quote arrives");
        calls += 1;
        if last == 10.0 {
            break;
        }
    }
    (calls, subscription.get_aggregation_period_info().unwrap())
}

#[test]
fn an_aggregation_period_conflates_the_batches() {
    let (plain, _) = batches(None);
    let (aggregated, info) = batches(Some(TimePeriod::value_of_millis(2000)));
    assert!(
        aggregated < plain,
        "{aggregated} batches with aggregation, {plain} without"
    );
    assert!(!info.is_unknown(), "the info is known once events arrive: {info}");
    assert_eq!(info.get_max(), 2000);
}

/// A change listener that sends the thread of each `configuration_changed` call.
struct ConfigurationListener(mpsc::Sender<thread::ThreadId>);

impl ObservableSubscriptionChangeListener for ConfigurationListener {
    fn symbols_added(&mut self, _symbols: &[Symbol]) {}

    fn configuration_changed(&mut self) {
        let _ = self.0.send(thread::current().id());
    }
}

/// Another aggregation period changes the configuration, on the calling thread before the call returns; the same
/// period and the executor do not, as in Java.
#[test]
fn another_aggregation_period_changes_the_configuration() {
    let hub = Hub::new();
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    let (sender, receiver) = mpsc::channel();
    subscription.add_change_listener(ConfigurationListener(sender)).unwrap();
    assert!(receiver.try_recv().is_err());

    let period = Some(TimePeriod::value_of("1s").unwrap());
    subscription.set_aggregation_period(period).unwrap();
    assert_eq!(receiver.try_recv().unwrap(), thread::current().id());
    subscription.set_aggregation_period(period).unwrap();
    let executor = Executor::new_fixed_thread_pool(1, "configuration").unwrap();
    subscription.set_executor(Some(&executor)).unwrap();
    subscription.set_executor(None).unwrap();
    assert!(receiver.try_recv().is_err());

    subscription.set_aggregation_period(None).unwrap();
    assert_eq!(receiver.try_recv().unwrap(), thread::current().id());
}

/// Changes the aggregation period from its own calls: once when symbols come, and once more from the first
/// configuration call.
struct NestedConfigurationListener {
    subscription: Arc<OnceLock<Weak<DXFeedSubscription<Quote>>>>,
    calls: mpsc::Sender<thread::ThreadId>,
    configured: bool,
}

impl NestedConfigurationListener {
    fn set_period(&self, period: &str) {
        let subscription = self.subscription.get().and_then(Weak::upgrade).unwrap();
        subscription
            .set_aggregation_period(Some(TimePeriod::value_of(period).unwrap()))
            .unwrap();
    }
}

impl ObservableSubscriptionChangeListener for NestedConfigurationListener {
    fn symbols_added(&mut self, _symbols: &[Symbol]) {
        self.set_period("1s");
    }

    fn configuration_changed(&mut self) {
        let _ = self.calls.send(thread::current().id());
        if !self.configured {
            self.configured = true;
            self.set_period("2s");
        }
    }
}

/// A configuration change made inside a call of the listener reaches it right after that call, on the same thread.
#[test]
fn a_nested_configuration_change_reaches_the_listener() {
    let hub = Hub::new();
    let subscription = Arc::new(hub.feed.create_subscription::<Quote>().unwrap());
    let cell = Arc::new(OnceLock::new());
    let (calls, receiver) = mpsc::channel();
    let listener = NestedConfigurationListener {
        subscription: Arc::clone(&cell),
        calls,
        configured: false,
    };
    subscription.add_change_listener(listener).unwrap();
    cell.set(Arc::downgrade(&subscription)).unwrap();

    subscription.add_symbols(["AAPL"]).unwrap();
    let me = thread::current().id();
    assert_eq!(receiver.recv_timeout(Duration::from_secs(5)).unwrap(), me);
    assert_eq!(receiver.recv_timeout(Duration::from_secs(5)).unwrap(), me);
    assert!(receiver.recv_timeout(Duration::from_millis(200)).is_err());
    assert_eq!(
        subscription.get_aggregation_period().unwrap(),
        Some(TimePeriod::value_of("2s").unwrap())
    );
}

/// A closure listener ignores configuration changes.
#[test]
fn a_closure_listener_ignores_configuration_changes() {
    let hub = Hub::new();
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    subscription.add_change_listener(|_: &[Symbol]| {}).unwrap();
    subscription
        .set_aggregation_period(Some(TimePeriod::value_of("1s").unwrap()))
        .unwrap();
}
