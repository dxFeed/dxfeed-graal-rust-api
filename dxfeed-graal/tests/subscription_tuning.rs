//! The tuning of a subscription: aggregation period, events batch limit, executor and controller.

mod support;

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use dxfeed_graal::event::Quote;
use dxfeed_graal::{DXFeedSubscription, Error, Executor, TimePeriod, TimePeriodInfo};
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
