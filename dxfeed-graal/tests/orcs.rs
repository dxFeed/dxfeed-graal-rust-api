//! The price level checker on orders built here, and the price level service at an address where nothing listens; no
//! network needed. The reading of `AuthOrderSource` needs a live service and is not tested here.

use std::time::{Duration, Instant};

use dxfeed_graal::event::{CandleSymbol, Order, OrderSource, Side};
use dxfeed_graal::orcs::{PriceLevelChecker, PriceLevelService};
use dxfeed_graal::{Error, Result, system};

const MINUTE: Duration = Duration::from_secs(60);

/// An order of the side at the price, at `time` milliseconds.
fn order(index: i64, side: Side, price: f64, time: i64) -> Order {
    let mut order = Order::new("AAPL");
    order.index = index;
    order.set_time(time);
    order.set_order_side(side);
    order.price = price;
    order.size = 100.0;
    order
}

#[test]
fn an_empty_list_is_valid() {
    assert!(PriceLevelChecker::validate(&[], MINUTE, false).unwrap());
}

#[test]
fn a_bid_below_the_ask_is_valid() {
    let orders = [
        order(1, Side::Buy, 9.0, 1_000),
        order(2, Side::Sell, 10.0, 2_000),
        order(3, Side::Buy, 5.0, 3_000),
    ];
    assert!(PriceLevelChecker::validate(&orders, MINUTE, false).unwrap());
}

#[test]
fn a_bid_above_the_ask_is_not_valid() {
    // The checker looks at the book of a moment when the next moment comes.
    let orders = [
        order(1, Side::Buy, 11.0, 1_000),
        order(2, Side::Sell, 10.0, 2_000),
        order(3, Side::Buy, 5.0, 3_000),
    ];
    assert!(!PriceLevelChecker::validate(&orders, MINUTE, true).unwrap());
}

#[test]
fn a_gap_above_the_bound_is_not_valid() {
    let orders = [
        order(1, Side::Buy, 9.0, 1_000),
        order(2, Side::Sell, 10.0, 1_000 + 2 * 60_000),
    ];
    assert!(!PriceLevelChecker::validate(&orders, MINUTE, false).unwrap());
    assert!(PriceLevelChecker::validate(&orders, 3 * MINUTE, false).unwrap());
}

#[test]
fn an_unreachable_service_reports_errors() {
    // A request waits for the connection a minute by default.
    system::set_property("com.devexperts.rmi.RequestSendingTimeout", "1000").unwrap();
    let service = PriceLevelService::new("127.0.0.1:1").unwrap();
    let symbol = CandleSymbol::value_of("AAPL{=d,gr=s}").unwrap();
    let start = Instant::now();
    assert_rmi_error(service.get_orders(&symbol, OrderSource::NTV, 0, 1, "test"));
    assert_rmi_error(service.get_quotes(&symbol, 0, 1, "test"));
    assert_rmi_error(service.get_auth_order_source());
    assert!(start.elapsed() < Duration::from_secs(30), "{:?}", start.elapsed());

    assert!(!service.is_closed());
    service.close().unwrap();
    service.close().unwrap();
    assert!(service.is_closed());
    assert!(service.get_quotes(&symbol, 0, 1, "test").is_err());
}

/// Asserts that a request failed in the RMI layer, not on its arguments.
fn assert_rmi_error<T: std::fmt::Debug>(result: Result<T>) {
    match result {
        Err(Error::Java(e)) => assert_eq!(e.get_class_name(), "com.devexperts.rmi.RuntimeRMIException"),
        other => panic!("expected RuntimeRMIException, got {other:?}"),
    }
}

#[test]
fn a_malformed_address_is_an_error() {
    assert!(PriceLevelService::new("127.0.0.1:1[").is_err());
}
