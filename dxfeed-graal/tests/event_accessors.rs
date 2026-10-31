//! The accessors every event has, owned or borrowed, as Java's `EventType`, `IndexedEvent` and `TimeSeriesEvent`:
//! offline and over `LOCAL_HUB` endpoints, no network needed.

mod support;

use std::collections::HashSet;
use std::sync::mpsc;

use dxfeed_graal::event::{BaseEvent, IndexedEvent, Message, Order, Quote, TimeAndSale, TimeSeriesEvent, Trade};
use dxfeed_graal::{Event, EventRef, EventType};
use support::{Hub, all_events, candle, candle_symbol, message, order, order_base, quote, time_and_sale};

/// The Java classes that implement `MarketEvent`.
const MARKET: &[&str] = &[
    "Quote",
    "Profile",
    "Summary",
    "Greeks",
    "Underlying",
    "TheoPrice",
    "Trade",
    "TradeETH",
    "TimeAndSale",
    "OrderBase",
    "Order",
    "AnalyticOrder",
    "OtcMarketsOrder",
    "SpreadOrder",
    "Series",
    "OptionSale",
    "MarketMaker",
    "NuamOrder",
    "NuamTimeAndSale",
    "NuamTrade",
    "OrderImbalance",
];

/// The Java classes that implement `LastingEvent`.
const LASTING: &[&str] = &[
    "Quote",
    "Profile",
    "Summary",
    "Greeks",
    "Candle",
    "DailyCandle",
    "Underlying",
    "TheoPrice",
    "Trade",
    "TradeETH",
    "Configuration",
    "TextConfiguration",
    "NuamTrade",
];

/// The Java classes that implement `IndexedEvent`.
const INDEXED: &[&str] = &[
    "Greeks",
    "Candle",
    "DailyCandle",
    "Underlying",
    "TheoPrice",
    "TimeAndSale",
    "OrderBase",
    "Order",
    "AnalyticOrder",
    "OtcMarketsOrder",
    "SpreadOrder",
    "Series",
    "OptionSale",
    "MarketMaker",
    "NuamOrder",
    "NuamTimeAndSale",
];

/// The Java classes that implement `TimeSeriesEvent`.
const TIME_SERIES: &[&str] = &[
    "Greeks",
    "Candle",
    "DailyCandle",
    "Underlying",
    "TheoPrice",
    "TimeAndSale",
    "NuamTimeAndSale",
];

/// The order types: `OrderBase` and the classes Java derives from it.
const ORDERS: &[&str] = &[
    "OrderBase",
    "Order",
    "AnalyticOrder",
    "OtcMarketsOrder",
    "SpreadOrder",
    "NuamOrder",
];

/// One owned event of each of the 27 types.
fn every_event() -> Vec<Event> {
    let mut events = all_events("AAPL");
    events.push(order_base("AAPL").into());
    events
}

/// The mismatches between the trait objects an event gives and the Java interfaces of its type.
fn trait_mismatches(name: &str, actual: [bool; 5]) -> Vec<String> {
    let expected = [
        MARKET.contains(&name),
        LASTING.contains(&name),
        INDEXED.contains(&name),
        TIME_SERIES.contains(&name),
        ORDERS.contains(&name),
    ];
    if expected == actual {
        Vec::new()
    } else {
        vec![format!(
            "{name}: market, lasting, indexed, time series, order are {actual:?}, expected {expected:?}"
        )]
    }
}

/// The index of any indexed event, owned or borrowed.
fn index_of<E: IndexedEvent>(event: &E) -> i64 {
    event.index()
}

/// The symbol of any event, owned or borrowed.
fn symbol_of<E: BaseEvent>(event: &E) -> String {
    event.symbol().into_owned()
}

#[test]
fn owned_events_answer_the_accessors() {
    let events = every_event();
    let types: HashSet<EventType> = events.iter().map(Event::event_type).collect();
    assert_eq!(types.len(), EventType::ALL.len());
    assert_eq!(events.len(), EventType::ALL.len());

    let mut mismatches = Vec::new();
    for event in &events {
        let name = event.event_type().name();
        assert!(event.symbol().starts_with("AAPL"), "{event:?}");
        assert_eq!(BaseEvent::symbol(event), event.symbol());
        assert_eq!(BaseEvent::event_type(event), event.event_type());
        mismatches.extend(trait_mismatches(
            name,
            [
                event.as_market_event().is_some(),
                event.as_lasting_event().is_some(),
                event.as_indexed_event().is_some(),
                event.as_time_series_event().is_some(),
                event.as_order_event().is_some(),
            ],
        ));
        assert_eq!(event.index().is_some(), INDEXED.contains(&name), "{name}");
        assert_eq!(event.event_flags().is_some(), INDEXED.contains(&name), "{name}");
        assert_eq!(event.source().is_some(), INDEXED.contains(&name), "{name}");
        assert_eq!(event.time().is_some(), TIME_SERIES.contains(&name), "{name}");
    }
    assert!(mismatches.is_empty(), "{mismatches:#?}");
}

#[test]
fn indexed_values_come_from_the_event() {
    let quote: Event = quote("AAPL").into();
    assert_eq!(quote.index(), None);
    assert_eq!(quote.time(), None);

    let order = order("AAPL");
    let event: Event = order.clone().into();
    assert_eq!(event.index(), Some(order.index));
    assert_eq!(event.event_flags(), Some(order.event_flags));
    assert_eq!(event.source(), Some(order.source()));
    assert_eq!(event.time(), None);

    let sale = time_and_sale("AAPL");
    let event: Event = sale.clone().into();
    assert_eq!(event.index(), Some(sale.index));
    assert_eq!(event.time(), Some(TimeSeriesEvent::time(&sale)));

    let candle: Event = candle(&candle_symbol("AAPL")).into();
    assert!(candle.as_market_event().is_none());
    assert!(candle.as_time_series_event().is_some());
}

#[test]
fn setters_change_the_symbol_and_the_event_time() {
    let samples: Vec<Event> = vec![
        quote("AAPL").into(),
        order("AAPL").into(),
        time_and_sale("AAPL").into(),
        candle(&candle_symbol("AAPL")).into(),
        message("AAPL").into(),
    ];
    for mut event in samples {
        event.set_symbol("IBM");
        event.set_event_time(1_234);
        assert_eq!(event.symbol(), "IBM");
        assert_eq!(event.event_time(), 1_234);
        assert_eq!(BaseEvent::event_time(&event), 1_234);
    }
    let mut event: Event = quote("AAPL").into();
    event.set_symbol(String::from("MSFT"));
    assert_eq!(Quote::try_from(event).unwrap().symbol, "MSFT");
}

#[test]
fn downcast_ref_gives_the_owned_type() {
    let event: Event = quote("AAPL").into();
    assert_eq!(event.downcast_ref::<Quote>().unwrap().symbol, "AAPL");
    assert!(event.downcast_ref::<Trade>().is_none());
}

#[test]
fn generic_code_takes_owned_events() {
    assert_eq!(index_of(&order("AAPL")), order("AAPL").index);
    assert_eq!(symbol_of(&message("AAPL")), "AAPL");
    assert_eq!(symbol_of(&candle(&candle_symbol("AAPL"))), candle_symbol("AAPL"));
    let event: Event = order("AAPL").into();
    assert_eq!(symbol_of(&event), "AAPL");
}

#[test]
fn an_unmapped_event_has_no_values() {
    let event = EventRef::Other(-1);
    assert_eq!(event.symbol(), None);
    assert_eq!(event.event_time(), None);
    assert_eq!(event.event_type(), None);
    assert_eq!(event.index(), None);
    assert_eq!(event.event_flags(), None);
    assert_eq!(event.source(), None);
    assert_eq!(event.time(), None);
    assert!(event.as_market_event().is_none());
    assert!(event.as_order_event().is_none());
    assert!(event.downcast::<Quote>().is_none());
}

/// Publishes `events` and returns the types of the views the listener received and what it found wrong in them.
fn view_mismatches(events: &[Event]) -> (Vec<EventType>, Vec<String>) {
    let hub = Hub::new();
    let subscription = hub.feed.create_subscription_with_types(EventType::ALL).unwrap();
    let (sender, receiver) = mpsc::channel();
    subscription
        .add_event_listener(move |events| {
            for event in events {
                let _ = sender.send(check_view(event));
            }
        })
        .unwrap();
    subscription
        .add_symbols(["AAPL".to_owned(), candle_symbol("AAPL")])
        .unwrap();
    hub.publish_and_process(events);
    drop(subscription);
    let mut types = Vec::new();
    let mut mismatches = Vec::new();
    for (event_type, mut found) in receiver.try_iter() {
        types.push(event_type);
        mismatches.append(&mut found);
    }
    (types, mismatches)
}

/// Compares the accessors of a received view with those of its owned copy.
fn check_view(event: EventRef<'_>) -> (EventType, Vec<String>) {
    let owned = event.to_owned().unwrap();
    let name = owned.event_type().name();
    let mut mismatches = Vec::new();
    let mut expect = |what: &str, same: bool| {
        if !same {
            mismatches.push(format!("{name}: {what}"));
        }
    };
    expect("symbol", event.symbol().as_deref() == Some(owned.symbol()));
    expect("event time", event.event_time() == Some(owned.event_time()));
    expect("event type", event.event_type() == Some(owned.event_type()));
    expect("index", event.index() == owned.index());
    expect("event flags", event.event_flags() == owned.event_flags());
    expect("source", event.source() == owned.source());
    expect("time", event.time() == owned.time());
    if let (Some(view), Some(copy)) = (event.as_order_event(), owned.as_order_event()) {
        expect("has size", view.has_size() == copy.has_size());
        expect("market maker", view.market_maker() == copy.market_maker());
        expect("price", view.price().to_bits() == copy.price().to_bits());
    }
    if let Some(order) = event.downcast::<Order>() {
        expect("generic index", index_of(order) == order.index());
        expect("generic symbol", symbol_of(order) == owned.symbol());
    }
    if let Some(message) = event.downcast::<Message>() {
        expect("message symbol", symbol_of(message) == owned.symbol());
    }
    expect(
        "downcast to a wrong type",
        name == "Trade" || event.downcast::<Trade>().is_none(),
    );
    expect(
        "downcast to its type",
        name != "TimeAndSale" || event.downcast::<TimeAndSale>().is_some(),
    );
    let mut traits = trait_mismatches(
        name,
        [
            event.as_market_event().is_some(),
            event.as_lasting_event().is_some(),
            event.as_indexed_event().is_some(),
            event.as_time_series_event().is_some(),
            event.as_order_event().is_some(),
        ],
    );
    mismatches.append(&mut traits);
    (owned.event_type(), mismatches)
}

#[test]
fn views_answer_the_same_as_their_copies() {
    // The hub does not publish an `OrderBase`; `traits.rs` checks its view at compile time.
    let events = all_events("AAPL");
    let (received, mismatches) = view_mismatches(&events);
    let received: HashSet<EventType> = received.into_iter().collect();
    let sent: HashSet<EventType> = events.iter().map(Event::event_type).collect();
    assert_eq!(received, sent);
    assert!(mismatches.is_empty(), "{mismatches:#?}");
}

#[test]
fn order_views_tell_whether_they_have_a_size() {
    let orders: Vec<Event> = [0.0, f64::NAN, 1.0]
        .into_iter()
        .enumerate()
        .map(|(i, size)| {
            let mut order = order("AAPL");
            order.index += i as i64;
            order.size = size;
            Event::from(order)
        })
        .collect();
    let has_size: Vec<bool> = orders.iter().map(|e| e.as_order_event().unwrap().has_size()).collect();
    assert_eq!(has_size, [false, false, true]);
    let (received, mismatches) = view_mismatches(&orders);
    assert_eq!(received, [EventType::Order; 3]);
    assert!(mismatches.is_empty(), "{mismatches:#?}");
}
