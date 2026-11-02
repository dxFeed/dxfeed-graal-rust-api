//! The price level checker on orders built here, and the price level service at an address where nothing listens; no
//! network needed. An `AuthOrderSource` created here reads back its entries; reading one from a live service is not
//! tested here.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use dxfeed_graal::event::{CandleSymbol, Order, OrderSource, Side};
use dxfeed_graal::orcs::{AuthOrderSource, PriceLevelChecker, PriceLevelService};
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
    assert_rmi_error(service.get_orders(&symbol, OrderSource::NTV, 0, 1, Some("test")));
    assert_rmi_error(service.get_quotes(&symbol, 0, 1, Some("test")));
    // Without a caller, the requests go out with the default caller of the SDK.
    assert_rmi_error(service.get_orders(&symbol, OrderSource::NTV, 0, 1, None));
    assert_rmi_error(service.get_quotes(&symbol, 0, 1, None));
    assert!(matches!(
        service.get_quotes(&symbol, 0, 1, Some("te\0st")),
        Err(Error::InvalidArgument(_))
    ));
    assert_rmi_error(service.get_auth_order_source());
    assert!(start.elapsed() < Duration::from_secs(30), "{:?}", start.elapsed());

    assert!(!service.is_closed());
    service.close().unwrap();
    service.close().unwrap();
    assert!(service.is_closed());
    assert!(service.get_quotes(&symbol, 0, 1, None).is_err());
}

/// Asserts that a request failed in the RMI layer, not on its arguments.
fn assert_rmi_error<T: std::fmt::Debug>(result: Result<T>) {
    match result {
        Err(Error::Java(e)) => assert_eq!(e.get_class_name(), "com.devexperts.rmi.RuntimeRMIException"),
        other => panic!("expected RuntimeRMIException, got {other:?}"),
    }
}

#[test]
fn the_granularity_key_is_the_java_one() {
    assert_eq!(PriceLevelService::GRANULARITY_ATTRIBUTE_KEY, "gr");
    let symbol = format!("AAPL{{=d,{}=s}}", PriceLevelService::GRANULARITY_ATTRIBUTE_KEY);
    assert_eq!(CandleSymbol::value_of(&symbol).unwrap().to_string(), "AAPL{=d,gr=s}");
}

#[test]
fn a_malformed_address_is_an_error() {
    assert!(PriceLevelService::new("127.0.0.1:1[").is_err());
}

#[test]
fn a_created_auth_order_source_returns_its_entries() {
    let mut entries = HashMap::new();
    entries.insert(OrderSource::NTV.get_id(), vec!["AAPL".to_owned(), "IBM".to_owned()]);
    entries.insert(OrderSource::DEX.get_id(), Vec::new());
    let source = AuthOrderSource::create(&entries).unwrap();
    let mut read = source.get_by_ids().unwrap();
    for symbols in read.values_mut() {
        symbols.sort();
    }
    assert_eq!(read, entries);
    let by_sources = source.get_by_order_sources().unwrap();
    assert_eq!(by_sources.len(), 2);
    assert_eq!(by_sources[&OrderSource::DEX], Vec::<String>::new());

    assert!(
        AuthOrderSource::create(&HashMap::new())
            .unwrap()
            .get_by_ids()
            .unwrap()
            .is_empty()
    );
}

/// The symbols of a source are a set: duplicates are kept once.
#[test]
fn created_symbols_are_a_set() {
    let symbols = HashMap::from([(
        OrderSource::DEX.get_id(),
        vec!["MSFT".to_owned(), "AAPL".to_owned(), "MSFT".to_owned()],
    )]);
    let source = AuthOrderSource::create(&symbols).unwrap();
    let mut created = source.get_by_ids().unwrap().remove(&OrderSource::DEX.get_id()).unwrap();
    created.sort();
    assert_eq!(created, ["AAPL", "MSFT"]);
}

#[test]
fn a_symbol_with_a_nul_byte_is_invalid() {
    let entries = HashMap::from([(OrderSource::NTV.get_id(), vec!["A\0B".to_owned()])]);
    assert!(matches!(
        AuthOrderSource::create(&entries),
        Err(Error::InvalidArgument(_))
    ));
}
