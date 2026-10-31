//! The indexed event transactional model over `LOCAL_HUB` endpoints: snapshots and transactions of one source reach
//! the listener whole, and its subscription controller attaches and detaches it; no network needed.

mod support;

use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use dxfeed_graal::event::{EventFlags, Order, OrderSource, Side};
use dxfeed_graal::model::IndexedEventTxModel;
use dxfeed_graal::{Error, Symbol};
use support::Hub;

/// One listener call: the indices of the orders within their source and whether they are a snapshot.
type Call = (Vec<i64>, bool);

/// An order of the NTV book with the index and the flags.
fn order(index: i64, flags: EventFlags) -> Order {
    let mut order = Order {
        index,
        price: 100.0 + index as f64,
        size: 10.0,
        event_flags: flags,
        ..Order::new("EVENT")
    };
    order.set_source(OrderSource::NTV.id()).unwrap();
    order.set_order_side(Side::Buy);
    order
}

fn model(hub: Option<&Hub>) -> (IndexedEventTxModel<Order>, Receiver<Call>) {
    let (sender, receiver) = mpsc::channel();
    let mut builder = IndexedEventTxModel::<Order>::new_builder()
        .with_symbol("EVENT")
        .with_source(OrderSource::NTV)
        .with_listener(move |orders, is_snapshot| {
            let indices = orders.map(|order| order.index() & 0xFFFF_FFFF).collect();
            let _ = sender.send((indices, is_snapshot));
        });
    if let Some(hub) = hub {
        builder = builder.with_feed(&hub.feed);
    }
    (builder.build().unwrap(), receiver)
}

fn publish(hub: &Hub, orders: Vec<Order>) {
    let events: Vec<_> = orders.into_iter().map(Into::into).collect();
    hub.publisher.publish_events(&events).unwrap();
    hub.executor.process_all_pending().unwrap();
}

/// A snapshot of two orders; Java ends a snapshot with the order of index 0.
fn snapshot() -> Vec<Order> {
    vec![order(1, EventFlags::SNAPSHOT_BEGIN), order(0, EventFlags::SNAPSHOT_END)]
}

#[test]
fn snapshots_and_transactions_come_whole() {
    let hub = Hub::new();
    let (model, calls) = model(Some(&hub));
    hub.executor.process_all_pending().unwrap();
    assert_eq!(model.symbol().unwrap(), "EVENT");
    assert_eq!(model.source().unwrap(), OrderSource::NTV);
    assert!(!model.is_closed().unwrap());

    publish(&hub, snapshot());
    assert_eq!(calls.try_iter().collect::<Vec<_>>(), [(vec![1, 0], true)]);

    publish(
        &hub,
        vec![
            order(2, EventFlags::TX_PENDING),
            order(3, EventFlags::TX_PENDING),
            order(4, EventFlags::empty()),
        ],
    );
    assert_eq!(calls.try_iter().collect::<Vec<_>>(), [(vec![2, 3, 4], false)]);
}

#[test]
fn a_closed_model_gets_nothing() {
    let hub = Hub::new();
    let (model, calls) = model(Some(&hub));
    hub.executor.process_all_pending().unwrap();
    model.close().unwrap();
    model.close().unwrap();
    assert!(model.is_closed().unwrap());
    publish(&hub, snapshot());
    assert!(calls.try_recv().is_err());
}

#[test]
fn a_dropped_model_gets_nothing() {
    let hub = Hub::new();
    let (model, calls) = model(Some(&hub));
    hub.executor.process_all_pending().unwrap();
    drop(model);
    publish(&hub, snapshot());
    assert!(calls.try_recv().is_err());
}

#[test]
fn the_subscription_controller_attaches_and_detaches_the_model() {
    let hub = Hub::new();
    let (model, calls) = model(None);
    let controller = model.subscription_controller().unwrap();
    publish(&hub, snapshot());
    assert!(calls.try_recv().is_err(), "a model without a feed gets nothing");

    controller.attach(&hub.feed).unwrap();
    controller.attach(&hub.feed).unwrap();
    hub.executor.process_all_pending().unwrap();
    publish(
        &hub,
        vec![order(0, EventFlags::SNAPSHOT_BEGIN | EventFlags::SNAPSHOT_END)],
    );
    assert_eq!(calls.try_iter().last(), Some((vec![0], true)));

    controller.detach(&hub.feed).unwrap();
    hub.executor.process_all_pending().unwrap();
    publish(&hub, vec![order(5, EventFlags::empty())]);
    assert!(calls.try_recv().is_err(), "a detached model gets nothing");
}

#[test]
fn the_subscription_controller_changes_the_delivery() {
    let (model, _calls) = model(None);
    let controller = model.subscription_controller().unwrap();
    controller.set_events_batch_limit(7).unwrap();
    assert_eq!(controller.events_batch_limit().unwrap(), 7);
    controller.set_aggregation_period(Duration::from_millis(250)).unwrap();
    assert_eq!(controller.aggregation_period().unwrap(), Duration::from_millis(250));
}

#[test]
fn the_builder_sets_the_delivery() {
    let model = IndexedEventTxModel::<Order>::new_builder()
        .with_symbol("EVENT")
        .with_events_batch_limit(3)
        .with_aggregation_period(Duration::from_millis(100))
        .with_listener(|_, _| {})
        .build()
        .unwrap();
    assert_eq!(model.source().unwrap(), OrderSource::DEFAULT);
    let controller = model.subscription_controller().unwrap();
    assert_eq!(controller.events_batch_limit().unwrap(), 3);
    assert_eq!(controller.aggregation_period().unwrap(), Duration::from_millis(100));
}

#[test]
fn an_indexed_symbol_sets_the_source() {
    let build = |builder: dxfeed_graal::model::IndexedEventTxModelBuilder<Order>| {
        builder.with_listener(|_, _| {}).build().unwrap().source().unwrap()
    };
    let indexed = Symbol::indexed("EVENT", OrderSource::NTV);
    assert_eq!(
        build(IndexedEventTxModel::new_builder().with_symbol(indexed.clone())),
        OrderSource::NTV
    );
    let builder = IndexedEventTxModel::new_builder().with_source(OrderSource::ntv);
    assert_eq!(
        build(builder.with_symbol(indexed.clone())),
        OrderSource::NTV,
        "the last source wins"
    );
    let builder = IndexedEventTxModel::new_builder().with_symbol(indexed);
    assert_eq!(
        build(builder.with_source(OrderSource::ntv)),
        OrderSource::ntv,
        "the last source wins"
    );
}

#[test]
fn the_aggregation_period_is_zero_by_default() {
    let (model, _calls) = model(None);
    let controller = model.subscription_controller().unwrap();
    assert_eq!(controller.aggregation_period().unwrap(), Duration::ZERO);
}

#[test]
fn a_model_needs_a_symbol_and_a_listener() {
    let built = IndexedEventTxModel::<Order>::new_builder()
        .with_listener(|_, _| {})
        .build();
    assert!(matches!(built, Err(Error::InvalidArgument(_))), "{built:?}");
    let built = IndexedEventTxModel::<Order>::new_builder().with_symbol("EVENT").build();
    assert!(matches!(built, Err(Error::InvalidArgument(_))), "{built:?}");
}
