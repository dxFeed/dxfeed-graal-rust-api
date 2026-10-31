//! The sources of the indexed events as Java's `OrderSource` has them: the publication table, the typed source of the
//! orders and of `OrderImbalance`, and the sources created by `IndexedEventSource::new`. Offline, no network needed.

use std::collections::HashSet;

use dxfeed_graal::Error;
use dxfeed_graal::event::{
    Event, EventType, IndexedEvent, IndexedEventSource, Order, OrderImbalance, OrderSource, Quote,
};

/// The names of the sources in `sources`.
fn names(sources: &[OrderSource]) -> Vec<String> {
    sources.iter().map(|source| source.to_string()).collect()
}

#[test]
fn publishable_follows_the_java_table() {
    let publishable = |event_type| names(&OrderSource::publishable(event_type).unwrap());
    // The declarations of Java's `OrderSource` with `PUB_ORDER`, in order.
    let java = "DEFAULT NTV ntv NFX ESPD XNFI ICE ISE DEA DEX dex BYX BZX bzx BATE CHIX CEUX BXTR IST BI20 ABE FAIR GLBX \
                glbx ERIS XEUR xeur CFE C2OX SMFE smfe iex MEMX memx OCEA ocea pink ARCA arca CEDX cedx IGC igc EDX \
                edx NUAM nuam MOON moon OTCM otcm BOSS boss NEO neo";
    assert_eq!(publishable(EventType::Order), java.split(' ').collect::<Vec<_>>());
    assert_eq!(publishable(EventType::AnalyticOrder), ["DEFAULT", "GLBX"]);
    assert_eq!(publishable(EventType::OtcMarketsOrder), ["DEFAULT", "pink"]);
    assert_eq!(publishable(EventType::SpreadOrder), ["DEFAULT", "ISE"]);
    assert_eq!(publishable(EventType::NuamOrder), ["DEFAULT", "NUAM", "nuam"]);
    assert_eq!(
        publishable(EventType::OrderImbalance),
        ["DEFAULT", "NTV", "ARCA", "NUAM"]
    );
    assert_eq!(names(&OrderSource::full_order_book()), ["DEFAULT", "NTV", "NUAM"]);

    for event_type in [EventType::Quote, EventType::Candle, EventType::TimeAndSale] {
        assert!(matches!(
            OrderSource::publishable(event_type),
            Err(Error::InvalidArgument(_))
        ));
        assert!(OrderSource::NTV.is_publishable(event_type).is_err());
    }
}

#[test]
fn sources_tell_their_flags() {
    assert!(OrderSource::NTV.is_full_order_book());
    assert!(!OrderSource::COMPOSITE_BID.is_full_order_book());
    assert!(!OrderSource::ntv.is_full_order_book());
    assert!(OrderSource::ISE.is_publishable(EventType::SpreadOrder).unwrap());
    assert!(!OrderSource::ISE.is_publishable(EventType::AnalyticOrder).unwrap());
    assert!(!OrderSource::COMPOSITE_BID.is_publishable(EventType::Order).unwrap());
    let unknown = OrderSource::value_of("ZZZZ").unwrap();
    assert!(!unknown.is_publishable(EventType::Order).unwrap());
    assert!(!unknown.is_full_order_book());

    assert!(!OrderSource::is_special_source_id(0));
    assert!((1..=9).all(OrderSource::is_special_source_id));
    assert!(!OrderSource::is_special_source_id(10));
}

#[test]
fn orders_return_their_source() {
    let mut order = Order::new("AAPL");
    assert_eq!(order.source(), OrderSource::DEFAULT);
    order.set_source(OrderSource::NTV).unwrap();
    assert_eq!(order.source(), OrderSource::NTV);
    assert_eq!(IndexedEvent::source(&order), OrderSource::NTV);
    assert_eq!(order.source().to_string(), "NTV");
    assert_eq!(Event::Order(order.clone()).source(), Some(OrderSource::NTV));
    assert_eq!(Event::Quote(Quote::new("AAPL")).source(), None);

    let mut imbalance = OrderImbalance::new("AAPL");
    assert_eq!(imbalance.source(), OrderSource::DEFAULT);
    imbalance.set_source(OrderSource::ARCA);
    assert_eq!(
        (imbalance.source(), imbalance.source_id),
        (OrderSource::ARCA, OrderSource::ARCA.get_id())
    );
    assert!(imbalance.to_string().contains(", source=ARCA, "));
}

#[test]
fn new_sources_keep_their_names() {
    let custom = IndexedEventSource::new(0x0100_0000, "X");
    assert_eq!((custom.get_id(), custom.to_string()), (0x0100_0000, "X".to_owned()));
    assert_eq!(IndexedEventSource::from_id(0x0100_0000).unwrap(), custom);
    assert_eq!(IndexedEventSource::from_id(0x0100_0000).unwrap().get_name(), "X");

    // An order keeps the id only; the name comes back from the names given to `new`.
    let mut order = Order::new("AAPL");
    order.set_source(custom).unwrap();
    assert_eq!(order.source(), custom);
    assert!(order.to_string().contains(", source=X, "));

    // Equality and hashing use the id only.
    let renamed = IndexedEventSource::new(5, "Y");
    assert_eq!(renamed, OrderSource::AGGREGATE_BID);
    assert_eq!(
        (renamed.to_string(), OrderSource::AGGREGATE_BID.to_string()),
        ("Y".to_owned(), "AGGREGATE_BID".to_owned())
    );
    assert_eq!(IndexedEventSource::from_id(5).unwrap().get_name(), "AGGREGATE_BID");
    let set: HashSet<_> = [renamed, OrderSource::AGGREGATE_BID, custom].into_iter().collect();
    assert_eq!(set.len(), 2);

    // The first name of an id stays the name `from_id` gives.
    let other = IndexedEventSource::new(0x0100_0000, "Z");
    assert_eq!((other, other.get_name()), (custom, "Z".to_owned()));
    assert_eq!(IndexedEventSource::from_id(0x0100_0000).unwrap().get_name(), "X");

    assert!(IndexedEventSource::from_id(0x0200_0000).is_err());

    // An order index cannot keep an id whose bits 16 to 31 read as a special source.
    let mut order = Order::new("AAPL");
    let clash = IndexedEventSource::new(0x1_0000, "Clash");
    assert!(matches!(order.set_source(clash), Err(Error::InvalidArgument(_))));
    assert_eq!(order.source(), OrderSource::DEFAULT);
}
