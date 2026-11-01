//! The order book model over a `LOCAL_HUB` endpoint: the buy and sell lists are sorted by price, orders of a more
//! detailed scope hide the composite ones, the filter and the lot size apply as in Java, and closing or dropping the
//! model releases it; no network needed.

mod support;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, mpsc};

use dxfeed_graal::event::{Order, OrderSource, Scope, Side};
use dxfeed_graal::model::{OrderBookModel, OrderBookModelFilter};
use dxfeed_graal::{Error, live_handles};
use support::Hub;

/// Runs the tests of this file one by one, so that each can count the live handles.
fn serial() -> MutexGuard<'static, ()> {
    static SERIAL: Mutex<()> = Mutex::new(());
    SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
}

fn order(source: OrderSource, scope: Scope, side: Side, index: i64, price: f64, size: f64) -> Order {
    let mut order = Order {
        index,
        price,
        size,
        ..Order::new("BOOK")
    };
    order.set_source(source).unwrap();
    order.set_scope(scope);
    order.set_order_side(side);
    order
}

fn prices(orders: &[Order]) -> Vec<f64> {
    orders.iter().map(|order| order.price).collect()
}

fn sizes(orders: &[Order]) -> Vec<f64> {
    orders.iter().map(|order| order.size).collect()
}

#[test]
fn the_book_sorts_filters_and_scales_orders() {
    let _serial = serial();
    let hub = Hub::new();
    let before = live_handles();
    let model = OrderBookModel::new().unwrap();
    assert_eq!(model.get_filter().unwrap(), OrderBookModelFilter::All);
    assert_eq!(model.get_lot_size().unwrap(), 1);
    assert_eq!(model.get_symbol().unwrap(), None);
    assert!(model.get_executor().unwrap().is_none());
    model.set_executor(Some(&*hub.executor)).unwrap();
    assert!(model.get_executor().unwrap().is_some());
    model.set_executor(None).unwrap();
    assert!(model.get_executor().unwrap().is_none());
    model.set_symbol(Some("BOOK")).unwrap();
    assert_eq!(model.get_symbol().unwrap().as_deref(), Some("BOOK"));

    let (sender, changes) = mpsc::channel();
    let id = model
        .add_listener(move || {
            let _ = sender.send(());
        })
        .unwrap();
    let buys = model.get_buy_orders().unwrap();
    let sells = model.get_sell_orders().unwrap();
    let (sender, lists) = mpsc::channel();
    buys.add_listener(move |orders| {
        let _ = sender.send(orders.map(|order| order.price()).collect::<Vec<_>>());
    })
    .unwrap();
    model.attach(&hub.feed).unwrap();
    hub.executor.process_all_pending().unwrap();

    let events = [
        order(OrderSource::NTV, Scope::Order, Side::Buy, 1, 100.0, 10.0),
        order(OrderSource::NTV, Scope::Order, Side::Buy, 2, 101.0, 5.0),
        order(OrderSource::NTV, Scope::Order, Side::Sell, 3, 102.0, 7.0),
        order(OrderSource::DEFAULT, Scope::Composite, Side::Buy, 4, 99.0, 2.0),
    ];
    hub.publish_and_process(&events.clone().map(Into::into));
    // The individual orders hide the composite one.
    assert_eq!(prices(&buys.to_vec().unwrap()), [101.0, 100.0]);
    assert_eq!(prices(&sells.to_vec().unwrap()), [102.0]);
    assert!(changes.try_iter().count() > 0);
    assert_eq!(lists.try_iter().last().unwrap(), [101.0, 100.0]);

    // The lot size multiplies the sizes of all orders but the individual ones.
    model.set_lot_size(100).unwrap();
    assert_eq!(model.get_lot_size().unwrap(), 100);
    hub.executor.process_all_pending().unwrap();
    assert_eq!(sizes(&buys.to_vec().unwrap()), [5.0, 10.0]);
    assert!(matches!(model.set_lot_size(0), Err(Error::Java(_))));

    model.set_filter(OrderBookModelFilter::Order).unwrap();
    hub.executor.process_all_pending().unwrap();
    assert_eq!(model.get_filter().unwrap(), OrderBookModelFilter::Order);
    assert_eq!(prices(&buys.to_vec().unwrap()), [101.0, 100.0]);
    assert_eq!(prices(&sells.to_vec().unwrap()), [102.0]);

    model.remove_listener(id).unwrap();
    assert!(matches!(model.remove_listener(id), Err(Error::InvalidArgument(_))));
    model.close().unwrap();
    model.close().unwrap();
    drop((buys, sells));
    drop(model);
    assert_eq!(live_handles(), before);
}

#[test]
fn filters_have_the_java_names() {
    let names: Vec<_> = OrderBookModelFilter::ALL
        .iter()
        .map(|filter| filter.to_string())
        .collect();
    assert_eq!(
        names,
        [
            "COMPOSITE",
            "REGIONAL",
            "AGGREGATE",
            "ORDER",
            "COMPOSITE_REGIONAL",
            "COMPOSITE_REGIONAL_AGGREGATE",
            "ALL"
        ]
    );
    assert_eq!(
        "COMPOSITE_REGIONAL".parse::<OrderBookModelFilter>().unwrap(),
        OrderBookModelFilter::CompositeRegional
    );
    assert!(matches!(
        "all".parse::<OrderBookModelFilter>(),
        Err(Error::InvalidArgument(_))
    ));
}

#[test]
fn the_composite_filter_shows_the_composite_order_scaled_by_the_lot_size() {
    let _serial = serial();
    let hub = Hub::new();
    let before = live_handles();
    let model = OrderBookModel::new().unwrap();
    model.set_filter(OrderBookModelFilter::Composite).unwrap();
    model.set_symbol(Some("BOOK")).unwrap();
    let buys = model.get_buy_orders().unwrap();
    model.attach(&hub.feed).unwrap();
    hub.executor.process_all_pending().unwrap();
    let events = [
        order(OrderSource::DEFAULT, Scope::Composite, Side::Buy, 4, 99.0, 2.0),
        order(OrderSource::NTV, Scope::Order, Side::Buy, 1, 100.0, 10.0),
    ];
    hub.publish_and_process(&events.clone().map(Into::into));
    assert_eq!(prices(&buys.to_vec().unwrap()), [99.0]);

    model.set_lot_size(100).unwrap();
    hub.executor.process_all_pending().unwrap();
    assert_eq!(sizes(&buys.to_vec().unwrap()), [200.0]);

    model.set_filter(OrderBookModelFilter::All).unwrap();
    hub.executor.process_all_pending().unwrap();
    let all = buys.to_vec().unwrap();
    assert_eq!((prices(&all), sizes(&all)), (vec![100.0], vec![10.0]));

    // Clearing the symbol empties the book; the list of a closed model keeps its last orders.
    model.set_symbol(None).unwrap();
    assert_eq!(model.get_symbol().unwrap(), None);
    assert!(buys.to_vec().unwrap().is_empty());
    model.set_symbol(Some("BOOK")).unwrap();
    hub.publish_and_process(&events.clone().map(Into::into));
    assert_eq!(prices(&buys.to_vec().unwrap()), [100.0]);
    model.detach(&hub.feed).unwrap();
    // Clearing the model unsubscribes it, as Java's `clear` is `setSymbol(null)`.
    model.clear().unwrap();
    assert!(buys.to_vec().unwrap().is_empty());
    assert_eq!(model.get_symbol().unwrap(), None);
    model.set_symbol(Some("BOOK")).unwrap();
    model.attach(&hub.feed).unwrap();
    hub.executor.process_all_pending().unwrap();
    hub.publish_and_process(&events.clone().map(Into::into));
    drop(model);
    assert_eq!(prices(&buys.to_vec().unwrap()), [100.0]);
    drop(buys);
    assert_eq!(live_handles(), before);
}

#[test]
fn a_change_made_in_a_listener_calls_it_once_more() {
    let _serial = serial();
    let hub = Hub::new();
    let model = Arc::new(OrderBookModel::new().unwrap());
    model.set_symbol(Some("BOOK")).unwrap();
    model.attach(&hub.feed).unwrap();
    hub.executor.process_all_pending().unwrap();
    hub.publish_and_process(&[order(OrderSource::DEFAULT, Scope::Composite, Side::Buy, 4, 99.0, 2.0).into()]);

    let calls = Arc::new(AtomicUsize::new(0));
    let (counter, weak) = (calls.clone(), Arc::downgrade(&model));
    model
        .add_listener(move || {
            if counter.fetch_add(1, Ordering::SeqCst) == 0
                && let Some(model) = weak.upgrade()
            {
                // Java notifies this change on this thread, in this call.
                model.set_lot_size(3).unwrap();
            }
        })
        .unwrap();
    // The setters notify the listeners on the calling thread.
    model.set_lot_size(2).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(sizes(&model.get_buy_orders().unwrap().to_vec().unwrap()), [6.0]);
    model.close().unwrap();
}
