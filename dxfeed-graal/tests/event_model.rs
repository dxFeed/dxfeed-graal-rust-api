//! The indexed and time-series event models over a `LOCAL_HUB` endpoint: the lists keep their order, the listener
//! sees each change, a new symbol clears the list, and closing or dropping a model releases it; no network needed.

mod support;

use std::sync::{Mutex, MutexGuard, PoisonError, mpsc};

use dxfeed_graal::event::{Candle, EventFlags, Order, OrderSource, Side};
use dxfeed_graal::model::{IndexedEventModel, TimeSeriesEventModel};
use dxfeed_graal::{Error, Symbol, live_handles};
use support::Hub;

/// Runs the tests of this file one by one, so that each can count the live handles.
fn serial() -> MutexGuard<'static, ()> {
    static SERIAL: Mutex<()> = Mutex::new(());
    SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
}

fn candle(symbol: &str, time: i64, close: f64) -> Candle {
    let mut candle = Candle {
        close,
        ..support::candle(symbol)
    };
    candle.set_time(time);
    candle
}

fn times(candles: &[Candle]) -> Vec<i64> {
    candles.iter().map(|candle| candle.time()).collect()
}

#[test]
fn a_time_series_model_keeps_candles_by_time() {
    let _serial = serial();
    let hub = Hub::new();
    let before = live_handles();
    let model = TimeSeriesEventModel::<Candle>::new().unwrap();
    assert_eq!(model.get_symbol().unwrap(), None);
    assert_eq!(model.get_from_time().unwrap(), i64::MAX);
    assert_eq!(model.get_size_limit().unwrap(), i32::MAX);
    assert!(model.get_executor().unwrap().is_none());
    model.set_symbol(Some("C{=d}")).unwrap();
    model.set_from_time(1_000).unwrap();
    assert_eq!(model.get_from_time().unwrap(), 1_000);
    assert_eq!(
        model.get_symbol().unwrap().map(|symbol| symbol.to_string()).as_deref(),
        Some("C{=d}")
    );

    let candles = model.get_events_list().unwrap();
    let (sender, changes) = mpsc::channel();
    candles
        .add_listener(move |candles| {
            let _ = sender.send(candles.map(|candle| candle.time()).collect::<Vec<_>>());
        })
        .unwrap();
    model.attach(&hub.feed).unwrap();
    hub.executor.process_all_pending().unwrap();

    hub.publish_and_process(&[
        candle("C{=d}", 2_000, 2.0).into(),
        candle("C{=d}", 4_000, 4.0).into(),
        candle("C{=d}", 3_000, 3.0).into(),
    ]);
    assert_eq!(times(&candles.to_vec().unwrap()), [2_000, 3_000, 4_000]);
    let seen: Vec<_> = changes.try_iter().collect();
    assert_eq!(seen, [[2_000, 3_000, 4_000]], "one call for the batch");

    model.set_size_limit(2).unwrap();
    assert_eq!(model.get_size_limit().unwrap(), 2);
    hub.executor.process_all_pending().unwrap();
    assert_eq!(times(&candles.to_vec().unwrap()), [3_000, 4_000]);
    assert_eq!(changes.try_iter().count(), 0, "the limit does not call the listener");
    assert!(matches!(model.set_size_limit(-1), Err(Error::Java(_))));

    model.set_symbol(Some("D{=d}")).unwrap();
    // A new symbol calls the listener on this thread.
    assert_eq!(changes.try_iter().last(), Some(Vec::new()));
    hub.executor.process_all_pending().unwrap();
    assert!(candles.to_vec().unwrap().is_empty());
    model.set_symbol(None::<Symbol>).unwrap();
    assert_eq!(model.get_symbol().unwrap(), None);

    model.close().unwrap();
    model.close().unwrap();
    drop(candles);
    drop(model);
    assert_eq!(live_handles(), before);
}

#[test]
fn a_new_time_needs_an_unsubscribed_model_and_clear_resets_it() {
    let _serial = serial();
    let hub = Hub::new();
    let before = live_handles();
    let model = TimeSeriesEventModel::<Candle>::new().unwrap();
    model.set_symbol(Some("T{=d}")).unwrap();
    model.set_from_time(1_000).unwrap();
    let candles = model.get_events_list().unwrap();
    model.attach(&hub.feed).unwrap();
    hub.executor.process_all_pending().unwrap();
    hub.publish_and_process(&[candle("T{=d}", 2_000, 2.0).into(), candle("T{=d}", 3_000, 3.0).into()]);
    assert_eq!(times(&candles.to_vec().unwrap()), [2_000, 3_000]);

    // As in Java, the new time of a subscribed model keeps its subscription and its events.
    model.set_from_time(2_500).unwrap();
    assert_eq!(model.get_from_time().unwrap(), 2_500);
    hub.executor.process_all_pending().unwrap();
    assert_eq!(times(&candles.to_vec().unwrap()), [2_000, 3_000]);
    model.set_from_time(i64::MAX).unwrap();
    assert!(candles.to_vec().unwrap().is_empty());

    model.set_from_time(1_000).unwrap();
    model.clear().unwrap();
    assert_eq!(model.get_from_time().unwrap(), i64::MAX);
    assert_eq!(model.get_symbol().unwrap(), None);

    // A symbol without a time keeps the symbol but receives nothing.
    model.set_symbol(Some("T{=d}")).unwrap();
    assert!(model.get_symbol().unwrap().is_some());
    hub.publish_and_process(&[candle("T{=d}", 2_000, 2.0).into()]);
    assert!(candles.to_vec().unwrap().is_empty());

    // The list outlives its model and keeps its last events.
    model.set_from_time(1_000).unwrap();
    hub.executor.process_all_pending().unwrap();
    hub.publish_and_process(&[candle("T{=d}", 4_000, 4.0).into()]);
    assert_eq!(times(&candles.to_vec().unwrap()), [4_000]);
    drop(model);
    assert_eq!(times(&candles.to_vec().unwrap()), [4_000]);
    drop(candles);
    assert_eq!(live_handles(), before);
}

#[test]
fn an_indexed_model_keeps_orders_by_source_and_index() {
    let _serial = serial();
    let hub = Hub::new();
    let before = live_handles();
    let model = IndexedEventModel::<Order>::new().unwrap();
    model.set_symbol(Some("IDX")).unwrap();
    assert_eq!(model.get_symbol().unwrap(), Some(Symbol::from("IDX")));
    let orders = model.get_events_list().unwrap();
    model.attach(&hub.feed).unwrap();
    hub.executor.process_all_pending().unwrap();

    let order = |source: OrderSource, index: i64, flags: EventFlags| {
        let mut order = Order {
            index,
            price: 1.0,
            size: 1.0,
            event_flags: flags,
            ..Order::new("IDX")
        };
        order.set_source(source).unwrap();
        order.set_order_side(Side::Buy);
        order
    };
    let ntv = OrderSource::NTV;
    hub.publish_and_process(&[
        order(ntv, 3, EventFlags::empty()).into(),
        order(OrderSource::DEFAULT, 9, EventFlags::empty()).into(),
        order(ntv, 1, EventFlags::empty()).into(),
        order(ntv, 2, EventFlags::empty()).into(),
    ]);
    let keys = || -> Vec<(i32, i64)> {
        let orders = orders.to_vec().unwrap();
        orders
            .iter()
            .map(|order| (order.source().get_id(), order.index & 0xFFFF_FFFF))
            .collect()
    };
    let ntv_id = ntv.get_id();
    assert_eq!(keys(), [(0, 9), (ntv_id, 1), (ntv_id, 2), (ntv_id, 3)]);

    // A removed order leaves the list.
    hub.publish_and_process(&[order(ntv, 2, EventFlags::REMOVE_EVENT).into()]);
    assert_eq!(keys(), [(0, 9), (ntv_id, 1), (ntv_id, 3)]);

    // As in Java, an indexed subscription symbol never equals the event symbol of the orders.
    model.set_symbol(Some(Symbol::indexed("IDX", ntv))).unwrap();
    hub.publish_and_process(&[order(ntv, 4, EventFlags::empty()).into()]);
    assert!(orders.to_vec().unwrap().is_empty());

    model.clear().unwrap();
    assert_eq!(model.get_symbol().unwrap(), None);
    drop(orders);
    drop(model);
    assert_eq!(live_handles(), before);
}
