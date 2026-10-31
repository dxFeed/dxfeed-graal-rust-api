//! The market depth model over `LOCAL_HUB` endpoints: orders published with event flags give the sorted sides of the
//! book; no network needed.

mod support;

use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use dxfeed_graal::event::{EventFlags, IndexedEventSource, Order, OrderSource, Scope, Side};
use dxfeed_graal::model::MarketDepthModel;
use support::Hub;

/// One listener call: the prices of the buy side and of the sell side.
type Sides = (Vec<f64>, Vec<f64>);

/// A price level of the NTV book.
fn order(index: i64, side: Side, price: f64, size: f64, flags: EventFlags) -> Order {
    order_of(OrderSource::NTV, index, side, price, size, flags)
}

/// A price level of the book of `source`.
fn order_of(source: IndexedEventSource, index: i64, side: Side, price: f64, size: f64, flags: EventFlags) -> Order {
    let mut order = Order {
        index,
        price,
        size,
        event_flags: flags,
        ..Order::new("DEPTH")
    };
    order.set_source(source).unwrap();
    order.set_order_side(side);
    order.set_scope(Scope::Aggregate);
    order
}

fn model(hub: &Hub, depth_limit: usize, aggregation_period: Duration) -> (MarketDepthModel<Order>, Receiver<Sides>) {
    let (sender, receiver) = mpsc::channel();
    let model = MarketDepthModel::<Order>::new_builder()
        .with_feed(&hub.feed)
        .with_symbol("DEPTH")
        .with_sources([OrderSource::NTV])
        .with_depth_limit(depth_limit)
        .with_aggregation_period(aggregation_period)
        .with_listener(move |buy, sell| {
            let prices = |orders: &[Order]| orders.iter().map(|order| order.price).collect();
            let _ = sender.send((prices(buy), prices(sell)));
        })
        .build()
        .unwrap();
    hub.executor.process_all_pending().unwrap();
    (model, receiver)
}

fn publish(hub: &Hub, orders: Vec<Order>) {
    let events: Vec<_> = orders.into_iter().map(Into::into).collect();
    hub.publisher.publish_events(&events).unwrap();
    hub.executor.process_all_pending().unwrap();
}

/// A snapshot of three buy and three sell levels, the best ones in the middle of the indices.
fn book() -> Vec<Order> {
    vec![
        order(5, Side::Buy, 100.0, 1.0, EventFlags::SNAPSHOT_BEGIN),
        order(4, Side::Buy, 101.0, 1.0, EventFlags::empty()),
        order(3, Side::Sell, 103.0, 1.0, EventFlags::empty()),
        order(2, Side::Sell, 102.0, 1.0, EventFlags::empty()),
        order(1, Side::Buy, 99.0, 1.0, EventFlags::empty()),
        order(0, Side::Sell, 104.0, 1.0, EventFlags::SNAPSHOT_END),
    ]
}

#[test]
fn sides_are_sorted_from_the_best_price() {
    let hub = Hub::new();
    let (model, calls) = model(&hub, 0, Duration::ZERO);
    publish(&hub, book());
    let snapshot: Vec<Sides> = calls.try_iter().collect();
    assert_eq!(snapshot, [(vec![101.0, 100.0, 99.0], vec![102.0, 103.0, 104.0])]);

    // A removal and a new order come in one call.
    publish(
        &hub,
        vec![
            order(
                4,
                Side::Buy,
                101.0,
                1.0,
                EventFlags::REMOVE_EVENT | EventFlags::TX_PENDING,
            ),
            order(6, Side::Sell, 101.5, 1.0, EventFlags::empty()),
        ],
    );
    let update: Vec<Sides> = calls.try_iter().collect();
    assert_eq!(update, [(vec![100.0, 99.0], vec![101.5, 102.0, 103.0, 104.0])]);

    // An order of zero size leaves the book.
    publish(&hub, vec![order(1, Side::Buy, 99.0, 0.0, EventFlags::empty())]);
    let update: Vec<Sides> = calls.try_iter().collect();
    assert_eq!(update, [(vec![100.0], vec![101.5, 102.0, 103.0, 104.0])]);

    model.close().unwrap();
    model.close().unwrap();
    publish(&hub, vec![order(7, Side::Buy, 100.5, 1.0, EventFlags::empty())]);
    assert!(calls.try_recv().is_err(), "a closed model gets nothing");
}

#[test]
fn the_depth_limit_cuts_the_sides() {
    let hub = Hub::new();
    let (model, calls) = model(&hub, 2, Duration::ZERO);
    assert_eq!(model.get_depth_limit(), 2);
    publish(&hub, book());
    let snapshot: Vec<Sides> = calls.try_iter().collect();
    assert_eq!(snapshot, [(vec![101.0, 100.0], vec![102.0, 103.0])]);

    // A new order beyond the limit does not call the listener.
    publish(&hub, vec![order(6, Side::Sell, 105.0, 1.0, EventFlags::empty())]);
    assert!(calls.try_recv().is_err(), "an order beyond the limit changed");

    model.set_depth_limit(3);
    assert_eq!(model.get_depth_limit(), 3);
    let wider: Vec<Sides> = calls.try_iter().collect();
    assert_eq!(wider, [(vec![101.0, 100.0, 99.0], vec![102.0, 103.0, 104.0])]);
    model.set_depth_limit(3);
    assert!(calls.try_recv().is_err(), "the same limit calls nothing");
}

#[test]
fn updates_wait_for_the_aggregation_period() {
    let hub = Hub::new();
    let (model, calls) = model(&hub, 0, Duration::from_secs(1));
    publish(&hub, book());
    assert_eq!(calls.try_iter().count(), 1, "a snapshot comes at once");

    publish(&hub, vec![order(6, Side::Buy, 100.5, 1.0, EventFlags::empty())]);
    publish(&hub, vec![order(7, Side::Buy, 100.25, 1.0, EventFlags::empty())]);
    assert!(calls.try_recv().is_err(), "the updates wait");
    let (buy, _) = calls.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(buy, [101.0, 100.5, 100.25, 100.0, 99.0], "both updates in one call");

    // A waiting call moves to a new period; zero makes it at once.
    model.set_aggregation_period(Duration::from_secs(3600));
    assert_eq!(model.get_aggregation_period(), Duration::from_secs(3600));
    publish(&hub, vec![order(8, Side::Sell, 101.75, 1.0, EventFlags::empty())]);
    assert!(calls.recv_timeout(Duration::from_millis(200)).is_err());
    model.set_aggregation_period(Duration::ZERO);
    let (_, sell) = calls.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(sell, [101.75, 102.0, 103.0, 104.0]);

    // Dropping the model with a waiting call stops its timer.
    model.set_aggregation_period(Duration::from_secs(3600));
    publish(&hub, vec![order(9, Side::Sell, 101.5, 1.0, EventFlags::empty())]);
    drop(model);
    assert!(calls.recv_timeout(Duration::from_millis(200)).is_err());
}

#[test]
fn price_levels_go_before_individual_orders() {
    let hub = Hub::new();
    let (sender, calls) = mpsc::channel();
    let _model = MarketDepthModel::<Order>::new_builder()
        .with_feed(&hub.feed)
        .with_symbol("DEPTH")
        .with_listener(move |buy, _| {
            let _ = sender.send(buy.iter().map(|order| order.index & 0xFFFF).collect::<Vec<_>>());
        })
        .build()
        .unwrap();
    hub.executor.process_all_pending().unwrap();
    let individual = |index: i64, flags: EventFlags| {
        let mut order = order(index, Side::Buy, 100.0, 1.0, flags);
        order.set_scope(Scope::Order);
        order
    };
    publish(
        &hub,
        vec![
            individual(4, EventFlags::SNAPSHOT_BEGIN),
            order(3, Side::Buy, 100.0, 5.0, EventFlags::empty()),
            individual(2, EventFlags::empty()),
            order(1, Side::Buy, 100.0, 10.0, EventFlags::empty()),
            order(0, Side::Buy, 101.0, 1.0, EventFlags::SNAPSHOT_END),
        ],
    );
    let snapshot: Vec<Vec<i64>> = calls.try_iter().collect();
    assert_eq!(snapshot, [vec![0, 1, 3, 2, 4]]);
}

#[test]
fn a_model_without_a_symbol_is_an_error() {
    let hub = Hub::new();
    let result = MarketDepthModel::<Order>::new_builder()
        .with_feed(&hub.feed)
        .with_listener(|_, _| {})
        .build();
    assert!(result.is_err(), "{result:?}");
}

#[test]
fn the_listener_can_change_the_model() {
    let hub = Hub::new();
    let (sender, calls) = mpsc::channel();
    let shared: Arc<OnceLock<MarketDepthModel<Order>>> = Arc::new(OnceLock::new());
    // A weak reference: the model owns the listener.
    let inner = Arc::downgrade(&shared);
    let model = MarketDepthModel::<Order>::new_builder()
        .with_feed(&hub.feed)
        .with_symbol("DEPTH")
        .with_listener(move |buy, _| {
            let _ = sender.send(buy.len());
            if let Some(model) = inner.upgrade().as_ref().and_then(|shared| shared.get()) {
                // The model calls the listener again after this call; closing it here does not wait for this call.
                if model.get_depth_limit() == 0 {
                    model.set_depth_limit(1);
                } else {
                    model.close().unwrap();
                }
            }
        })
        .build()
        .unwrap();
    assert!(shared.set(model).is_ok());
    hub.executor.process_all_pending().unwrap();
    publish(&hub, book());
    let lengths: Vec<usize> = calls.try_iter().collect();
    assert_eq!(lengths, [3, 1]);
    publish(&hub, vec![order(6, Side::Buy, 102.0, 1.0, EventFlags::empty())]);
    assert!(calls.try_recv().is_err(), "the listener closed the model");
}

#[test]
fn a_snapshot_replaces_the_orders_of_its_source_only() {
    let hub = Hub::new();
    let (sender, calls) = mpsc::channel();
    let _model = MarketDepthModel::<Order>::new_builder()
        .with_feed(&hub.feed)
        .with_symbol("DEPTH")
        .with_sources([OrderSource::NTV, OrderSource::ntv])
        .with_listener(move |buy, _| {
            let _ = sender.send(buy.iter().map(|order| order.price).collect::<Vec<_>>());
        })
        .build()
        .unwrap();
    hub.executor.process_all_pending().unwrap();
    publish(&hub, book());
    let both = EventFlags::SNAPSHOT_BEGIN | EventFlags::SNAPSHOT_END;
    publish(&hub, vec![order_of(OrderSource::ntv, 0, Side::Buy, 100.5, 1.0, both)]);
    assert_eq!(calls.try_iter().last().unwrap(), [101.0, 100.5, 100.0, 99.0]);

    publish(&hub, vec![order(0, Side::Buy, 98.0, 1.0, both)]);
    assert_eq!(
        calls.try_iter().last().unwrap(),
        [100.5, 98.0],
        "the NTV orders are replaced"
    );
}

#[test]
fn the_listener_can_drop_the_model() {
    let hub = Hub::new();
    let (sender, calls) = mpsc::channel();
    let slot: Arc<Mutex<Option<MarketDepthModel<Order>>>> = Arc::new(Mutex::new(None));
    let inner = Arc::clone(&slot);
    let model = MarketDepthModel::<Order>::new_builder()
        .with_feed(&hub.feed)
        .with_symbol("DEPTH")
        .with_listener(move |buy, _| {
            let _ = sender.send(buy.len());
            drop(inner.lock().unwrap().take());
        })
        .build()
        .unwrap();
    *slot.lock().unwrap() = Some(model);
    hub.executor.process_all_pending().unwrap();
    publish(&hub, book());
    assert_eq!(calls.try_iter().collect::<Vec<_>>(), [3]);
    assert!(slot.lock().unwrap().is_none());
    publish(&hub, vec![order(6, Side::Buy, 102.0, 1.0, EventFlags::empty())]);
    assert!(calls.try_recv().is_err(), "a dropped model gets nothing");
}
