//! Subscriptions created without a feed and attached to feeds later.

mod support;

use std::sync::mpsc::{self, Receiver};

use dxfeed_graal::event::{Candle, Quote};
use dxfeed_graal::{DXFeedSubscription, Error, EventType, Symbol};
use support::{Hub, quote_with_bid};

/// A standalone subscription to quotes of IBM whose listener sends the bid of each quote.
fn subscription() -> (DXFeedSubscription<Quote>, Receiver<f64>) {
    let subscription = DXFeedSubscription::<Quote>::new().unwrap();
    let (sender, bids) = mpsc::channel();
    subscription
        .add_event_listener(move |quotes| {
            for quote in quotes {
                let _ = sender.send(quote.bid_price());
            }
        })
        .unwrap();
    subscription.add_symbols(["IBM"]).unwrap();
    (subscription, bids)
}

#[test]
fn a_standalone_subscription_receives_while_attached() {
    let hub = Hub::new();
    let (subscription, bids) = subscription();
    hub.publish_and_process(&[quote_with_bid("IBM", 1.0).into()]);
    assert!(bids.try_recv().is_err(), "an unattached subscription gets nothing");

    subscription.attach(&hub.feed).unwrap();
    subscription.attach(&hub.feed).unwrap();
    hub.publish_and_process(&[quote_with_bid("IBM", 2.0).into()]);
    assert_eq!(bids.try_iter().collect::<Vec<_>>(), [2.0]);

    subscription.detach(&hub.feed).unwrap();
    hub.publish_and_process(&[quote_with_bid("IBM", 3.0).into()]);
    assert!(bids.try_recv().is_err(), "a detached subscription gets nothing");
    assert_eq!(subscription.get_symbols().unwrap(), [Symbol::from("IBM")]);
}

#[test]
fn a_subscription_receives_from_two_feeds() {
    let (first, second) = (Hub::new(), Hub::new());
    let (subscription, bids) = subscription();
    first.feed.attach_subscription(&subscription).unwrap();
    second.feed.attach_subscription(&subscription).unwrap();
    first.publish_and_process(&[quote_with_bid("IBM", 1.0).into()]);
    second.publish_and_process(&[quote_with_bid("IBM", 2.0).into()]);
    let mut received: Vec<_> = bids.try_iter().collect();
    received.sort_by(f64::total_cmp);
    assert_eq!(received, [1.0, 2.0]);

    first.feed.detach_subscription(&subscription).unwrap();
    first.publish_and_process(&[quote_with_bid("IBM", 3.0).into()]);
    second.publish_and_process(&[quote_with_bid("IBM", 4.0).into()]);
    assert_eq!(bids.try_iter().collect::<Vec<_>>(), [4.0]);
}

#[test]
fn detach_and_clear_detaches_and_keeps_the_symbols() {
    let hub = Hub::new();
    let (subscription, bids) = subscription();
    hub.feed.attach_subscription(&subscription).unwrap();
    hub.publish_and_process(&[quote_with_bid("IBM", 1.0).into()]);
    assert_eq!(bids.try_iter().collect::<Vec<_>>(), [1.0]);

    hub.feed.detach_subscription_and_clear(&subscription).unwrap();
    hub.publish_and_process(&[quote_with_bid("IBM", 2.0).into()]);
    assert!(
        bids.try_iter().all(|bid| bid != 2.0),
        "a detached subscription gets no new quotes"
    );
    assert_eq!(subscription.get_symbols().unwrap(), [Symbol::from("IBM")]);
}

#[test]
fn a_closed_subscription_is_not_attached() {
    let hub = Hub::new();
    let (subscription, bids) = subscription();
    subscription.close().unwrap();
    subscription.attach(&hub.feed).unwrap();
    hub.publish_and_process(&[quote_with_bid("IBM", 1.0).into()]);
    assert!(bids.try_recv().is_err(), "a closed subscription gets nothing");
}

#[test]
fn a_standalone_subscription_takes_several_types() {
    let subscription = DXFeedSubscription::new_with_types(&[EventType::Quote, EventType::Trade]).unwrap();
    let mut types = subscription.get_event_types().unwrap();
    types.sort_by_key(|event_type| event_type.name());
    assert_eq!(types, [EventType::Quote, EventType::Trade]);
    assert!(matches!(
        DXFeedSubscription::new_with_types(&[]),
        Err(Error::InvalidArgument(_))
    ));

    let hub = Hub::new();
    let (sender, received) = mpsc::channel();
    subscription
        .add_event_listener(move |events| {
            for event in events {
                let _ = sender.send(event.event_type());
            }
        })
        .unwrap();
    subscription.add_symbols(["IBM"]).unwrap();
    subscription.attach(&hub.feed).unwrap();
    hub.publish_and_process(&[quote_with_bid("IBM", 1.0).into()]);
    assert_eq!(received.try_iter().collect::<Vec<_>>(), [Some(EventType::Quote)]);
}

#[test]
fn a_time_series_subscription_keeps_its_from_time() {
    let hub = Hub::new();
    let candles = hub.feed.create_time_series_subscription::<Candle>().unwrap();
    assert_eq!(candles.get_from_time(), i64::MAX);
    candles.set_from_time(1000).unwrap();
    assert_eq!(candles.get_from_time(), 1000);

    let series = hub
        .feed
        .create_time_series_subscription_with_types(&[EventType::Candle, EventType::TimeAndSale])
        .unwrap();
    assert_eq!(series.get_event_types().unwrap().len(), 2);
    series.set_from_time(2000).unwrap();
    series.add_symbols(["IBM"]).unwrap();
    assert_eq!(series.get_from_time(), 2000);
    assert_eq!(
        series.get_decorated_symbols().unwrap(),
        [Symbol::time_series("IBM", 2000)]
    );
    assert!(matches!(
        hub.feed.create_time_series_subscription_with_types(&[]),
        Err(Error::InvalidArgument(_))
    ));
    assert!(matches!(
        hub.feed
            .create_time_series_subscription_with_types(&[EventType::Candle, EventType::Quote]),
        Err(Error::InvalidArgument(_))
    ));
}

#[test]
fn a_controller_does_not_attach_a_closed_subscription() {
    let hub = Hub::new();
    let (subscription, bids) = subscription();
    let controller = subscription.get_subscription_controller().unwrap();
    controller.attach(&hub.feed).unwrap();
    hub.publish_and_process(&[quote_with_bid("IBM", 1.0).into()]);
    assert_eq!(bids.try_iter().collect::<Vec<_>>(), [1.0]);
    controller.detach(&hub.feed).unwrap();

    subscription.close().unwrap();
    controller.attach(&hub.feed).unwrap();
    hub.publish_and_process(&[quote_with_bid("IBM", 2.0).into()]);
    assert!(bids.try_recv().is_err(), "a closed subscription gets nothing");
}
