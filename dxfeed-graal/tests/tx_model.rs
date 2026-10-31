//! Transactional models over `LOCAL_HUB` endpoints: transactions and snapshots published with event flags reach the
//! listener whole; no network needed.

mod support;

use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use dxfeed_graal::event::{EventFlags, IndexedEventSource, Order, OrderSource, Side, TimeAndSale};
use dxfeed_graal::model::{IndexedTxModel, SortOrder, TimeSeriesTxModel};
use support::Hub;

/// One listener call: the source, the indices and flags of the events, and whether they are a snapshot.
type Call = (IndexedEventSource, Vec<(i64, EventFlags)>, bool);

/// An order of the NTV book with the index (within the source) and the flags.
fn order(index: i64, flags: EventFlags) -> Order {
    let mut order = Order {
        index,
        price: 100.0 + index as f64,
        size: 10.0,
        event_flags: flags,
        ..Order::new("TX")
    };
    order.set_source(OrderSource::NTV).unwrap();
    order.set_order_side(Side::Buy);
    order
}

/// The index of `order` within its source.
fn local_index(index: i64) -> i64 {
    index & 0xFFFF_FFFF
}

fn indexed_model(hub: &Hub, snapshot_processing: bool) -> (IndexedTxModel<Order>, Receiver<Call>) {
    let (sender, receiver) = mpsc::channel();
    let model = IndexedTxModel::<Order>::new_builder()
        .with_feed(&hub.feed)
        .with_symbol("TX")
        .with_sources([OrderSource::NTV])
        .with_snapshot_processing(snapshot_processing)
        .with_listener(move |source, orders, is_snapshot| {
            let orders = orders
                .map(|order| (local_index(order.index()), order.event_flags()))
                .collect();
            let _ = sender.send((source, orders, is_snapshot));
        })
        .build()
        .unwrap();
    (model, receiver)
}

fn publish(hub: &Hub, orders: Vec<Order>) {
    let events: Vec<_> = orders.into_iter().map(Into::into).collect();
    hub.publisher.publish_events(&events).unwrap();
    hub.executor.process_all_pending().unwrap();
}

fn indices(call: &Call) -> Vec<i64> {
    call.1.iter().map(|(index, _)| *index).collect()
}

#[test]
fn snapshots_and_transactions_come_whole() {
    let hub = Hub::new();
    let (model, calls) = indexed_model(&hub, false);
    hub.executor.process_all_pending().unwrap();
    assert!(model.is_batch_processing().unwrap());
    assert!(!model.is_snapshot_processing().unwrap());
    assert_eq!(model.get_sources().unwrap(), [OrderSource::NTV]);

    publish(
        &hub,
        vec![
            order(3, EventFlags::SNAPSHOT_BEGIN),
            order(2, EventFlags::empty()),
            order(1, EventFlags::empty()),
            // Java ends a snapshot with the order of index 0.
            order(0, EventFlags::SNAPSHOT_END),
        ],
    );
    let snapshot: Vec<Call> = calls.try_iter().collect();
    assert_eq!(snapshot.len(), 1, "{snapshot:?}");
    assert_eq!(snapshot[0].0, OrderSource::NTV);
    assert!(snapshot[0].2, "a snapshot");
    assert_eq!(indices(&snapshot[0]), [3, 2, 1, 0]);

    // A transaction waits for its last event.
    publish(
        &hub,
        vec![order(4, EventFlags::TX_PENDING), order(6, EventFlags::TX_PENDING)],
    );
    assert!(calls.try_recv().is_err(), "a pending transaction is not delivered");
    publish(&hub, vec![order(5, EventFlags::empty())]);
    let update: Vec<Call> = calls.try_iter().collect();
    assert_eq!(update.len(), 1, "{update:?}");
    assert!(!update[0].2, "an update");
    assert_eq!(indices(&update[0]), [4, 6, 5]);

    model.close().unwrap();
    model.close().unwrap();
    publish(&hub, vec![order(6, EventFlags::empty())]);
    assert!(calls.try_recv().is_err(), "a closed model gets nothing");
}

#[test]
fn snapshot_processing_gives_the_resulting_state() {
    let hub = Hub::new();
    let (model, calls) = indexed_model(&hub, true);
    hub.executor.process_all_pending().unwrap();
    assert!(model.is_snapshot_processing().unwrap());

    publish(
        &hub,
        vec![
            order(3, EventFlags::SNAPSHOT_BEGIN),
            order(2, EventFlags::REMOVE_EVENT),
            order(1, EventFlags::empty()),
            order(0, EventFlags::SNAPSHOT_END),
        ],
    );
    let snapshot: Vec<Call> = calls.try_iter().collect();
    assert_eq!(snapshot.len(), 1, "{snapshot:?}");
    assert!(snapshot[0].2);
    assert_eq!(indices(&snapshot[0]), [3, 1, 0], "the removed order is dropped");
}

#[test]
fn sources_and_feeds_can_change() {
    let hub = Hub::new();
    let (model, calls) = indexed_model(&hub, false);
    model.set_sources([OrderSource::NTV, OrderSource::ntv]).unwrap();
    let mut sources = model.get_sources().unwrap();
    sources.sort_by_key(|source| source.get_id());
    assert_eq!(sources, [OrderSource::NTV, OrderSource::ntv]);
    model.set_aggregation_period(Duration::from_millis(0)).unwrap();

    model.detach(&hub.feed).unwrap();
    publish(&hub, vec![order(1, EventFlags::empty())]);
    assert!(calls.try_recv().is_err(), "a detached model gets nothing");
    model.attach(&hub.feed).unwrap();
    hub.executor.process_all_pending().unwrap();
    publish(&hub, vec![order(1, EventFlags::empty())]);
    let update: Vec<Call> = calls.try_iter().collect();
    assert_eq!(update.len(), 1, "{update:?}");
}

#[test]
fn a_dropped_model_gets_nothing() {
    let hub = Hub::new();
    let (model, calls) = indexed_model(&hub, false);
    hub.executor.process_all_pending().unwrap();
    drop(model);
    publish(&hub, vec![order(1, EventFlags::empty())]);
    assert!(calls.try_recv().is_err());
}

#[test]
fn without_batch_processing_each_transaction_comes_alone() {
    let hub = Hub::new();
    let (sender, calls) = mpsc::channel();
    // Built without a feed, then attached.
    let model = IndexedTxModel::<Order>::new_builder()
        .with_symbol("TX")
        .with_sources([OrderSource::NTV])
        .with_batch_processing(false)
        .with_listener(move |_, orders, _| {
            let _ = sender.send(orders.map(|order| local_index(order.index())).collect::<Vec<_>>());
        })
        .build()
        .unwrap();
    assert!(!model.is_batch_processing().unwrap());
    model.attach(&hub.feed).unwrap();
    hub.executor.process_all_pending().unwrap();
    publish(
        &hub,
        vec![order(0, EventFlags::SNAPSHOT_BEGIN | EventFlags::SNAPSHOT_END)],
    );
    assert_eq!(calls.try_iter().collect::<Vec<_>>(), [vec![0]], "the snapshot");
    // Both transactions arrive in one batch of the hub.
    publish(
        &hub,
        vec![
            order(1, EventFlags::TX_PENDING),
            order(2, EventFlags::empty()),
            order(3, EventFlags::empty()),
        ],
    );
    let calls: Vec<Vec<i64>> = calls.try_iter().collect();
    assert_eq!(calls, [vec![1, 2], vec![3]]);
}

#[test]
fn a_model_without_a_symbol_is_an_error() {
    let hub = Hub::new();
    let result = IndexedTxModel::<Order>::new_builder().with_feed(&hub.feed).build();
    assert!(result.is_err(), "{result:?}");
}

#[test]
fn time_series_come_from_a_time_on() {
    let hub = Hub::new();
    let (sender, receiver) = mpsc::channel();
    let model = TimeSeriesTxModel::<TimeAndSale>::new_builder()
        .with_feed(&hub.feed)
        .with_symbol("TX")
        // The hub ends a snapshot with the event at the time the subscription is from.
        .with_from_time(1_000)
        .with_sorting(SortOrder::Descending)
        .with_snapshot_processing(true)
        .with_listener(move |source, sales, is_snapshot| {
            let times: Vec<i64> = sales.map(|sale| sale.time()).collect();
            let _ = sender.send((source, times, is_snapshot));
        })
        .build()
        .unwrap();
    hub.executor.process_all_pending().unwrap();
    assert_eq!(model.get_from_time().unwrap(), 1_000);

    let sale = |time: i64, flags: EventFlags| {
        let mut sale = TimeAndSale {
            price: 10.0,
            size: 1.0,
            event_flags: flags,
            ..TimeAndSale::new("TX")
        };
        sale.set_time(time);
        sale.into()
    };
    hub.publisher
        .publish_events(&[
            sale(3_000, EventFlags::SNAPSHOT_BEGIN),
            sale(2_000, EventFlags::empty()),
            sale(1_000, EventFlags::SNAPSHOT_END),
        ])
        .unwrap();
    hub.executor.process_all_pending().unwrap();
    let calls: Vec<_> = receiver.try_iter().collect();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert_eq!(calls[0].0, IndexedEventSource::DEFAULT);
    assert_eq!(calls[0].1, [3_000, 2_000, 1_000], "sorted by descending time");
    assert!(calls[0].2);

    model.set_from_time(i64::MAX).unwrap();
    assert_eq!(model.get_from_time().unwrap(), i64::MAX);
}
