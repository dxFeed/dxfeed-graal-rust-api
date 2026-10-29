//! `Display` of events: the text of Java's `toString`. The expected strings are written from the Java formats.

mod support;

use std::sync::mpsc;

use dxfeed_graal::event::{Quote, Trade};
use dxfeed_graal::{Event, EventRef, EventType};
use support::{Hub, is_time, quote};

/// The value of `name=` in the text of an event.
fn field<'a>(text: &'a str, name: &str) -> &'a str {
    let start = text
        .find(&format!(", {name}="))
        .unwrap_or_else(|| panic!("no {name} in {text}"))
        + name.len()
        + 3;
    let rest = &text[start..];
    &rest[..rest.find([',', '}']).unwrap()]
}

#[test]
fn new_quote_prints_as_in_java() {
    assert_eq!(
        Quote::new("AAPL").to_string(),
        "Quote{AAPL, eventTime=0, time=0, timeNanoPart=0, sequence=0, bidTime=0, bidExchange=\\0, bidPrice=NaN, \
         bidSize=NaN, askTime=0, askExchange=\\0, askPrice=NaN, askSize=NaN}"
    );
}

#[test]
fn new_trade_prints_as_in_java() {
    assert_eq!(
        Trade::new("AAPL").to_string(),
        "Trade{AAPL, eventTime=0, time=0, timeNanoPart=0, sequence=0, exchange=\\0, tradeId=0, price=NaN, change=NaN, \
         size=NaN, day=19700101, dayVolume=NaN, dayTurnover=NaN, direction=UNDEFINED, ETH=false}"
    );
}

#[test]
fn quote_values_print_as_in_java() {
    let quote = Quote {
        time_millis_sequence: 5,
        time_nano_part: 7,
        bid_exchange_code: 'Q',
        bid_price: 100.0,
        bid_size: 1e7,
        ask_exchange_code: 'K',
        ask_price: 101.5,
        ask_size: 250.0,
        ..Quote::new("IBM")
    };
    assert_eq!(
        quote.to_string(),
        "Quote{IBM, eventTime=0, time=0, timeNanoPart=7, sequence=5, bidTime=0, bidExchange=Q, bidPrice=100.0, \
         bidSize=1.0E7, askTime=0, askExchange=K, askPrice=101.5, askSize=250.0}"
    );
}

#[test]
fn trade_values_print_as_in_java() {
    let trade = Trade {
        time_sequence: 9,
        exchange_code: '\u{e9}',
        price: 1.5e-5,
        change: -0.25,
        size: 12345678.9,
        day_id: 20000,
        day_volume: 0.0001,
        day_turnover: 1e21,
        flags: 4 << 1 | 1,
        trade_id: 42,
        ..Trade::new("MSFT")
    };
    assert_eq!(
        trade.to_string(),
        "Trade{MSFT, eventTime=0, time=0, timeNanoPart=0, sequence=9, exchange=\\u00e9, tradeId=42, price=1.5E-5, \
         change=-0.25, size=1.23456789E7, day=20241004, dayVolume=1.0E-4, dayTurnover=1.0E21, direction=ZERO_UP, \
         ETH=true}"
    );
}

#[test]
fn times_print_through_the_sdk() {
    let quote = Quote {
        event_time: 1_700_000_000_123,
        time_millis_sequence: 456 << 22,
        bid_time: 1_700_000_000_000,
        ask_time: 1_700_000_001_000,
        ..Quote::new("AAPL")
    };
    let text = quote.to_string();
    for name in ["eventTime", "time", "bidTime", "askTime"] {
        assert!(is_time(field(&text, name)), "{name} in {text}");
    }
    assert!(field(&text, "eventTime").ends_with(".123"), "{text}");
    assert!(field(&text, "time").ends_with(".456"), "{text}");
    assert!(field(&text, "bidTime").ends_with(".000"), "{text}");
    // Both times fall on one minute in every time zone: only the seconds differ.
    assert_eq!(field(&text, "bidTime")[..13], field(&text, "askTime")[..13]);

    let trade = Trade {
        time_sequence: (1_700_000_000_i64 << 32) | (789 << 22),
        ..Trade::new("AAPL")
    };
    let text = trade.to_string();
    assert!(is_time(field(&text, "time")), "{text}");
    assert!(field(&text, "time").ends_with(".789"), "{text}");
}

#[test]
fn events_print_as_their_payload() {
    let quote = Quote {
        bid_price: 1.0,
        ..Quote::new("AAPL")
    };
    assert_eq!(Event::from(quote.clone()).to_string(), quote.to_string());
    assert_eq!(
        Event::from(Trade::new("AAPL")).to_string(),
        Trade::new("AAPL").to_string()
    );
    assert_eq!(EventRef::Other(42).to_string(), "Event{class=42}");
}

#[test]
fn received_events_print_like_their_copies() {
    let hub = Hub::new();
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    let (sender, texts) = mpsc::channel();
    subscription
        .add_event_listener(move |quotes| {
            for quote in quotes {
                // Printing a time calls the SDK from inside the listener.
                let _ = sender.send((quote.to_string(), quote.to_owned().to_string()));
            }
        })
        .unwrap();
    subscription.add_symbols(["AAPL"]).unwrap();
    let sent = Quote {
        bid_time: 1_700_000_000_000,
        ..quote("AAPL")
    };
    hub.publish_and_process(&[sent.into()]);
    let (borrowed, owned) = texts.try_recv().expect("no quote");
    assert_eq!(borrowed, owned);
    assert!(is_time(field(&borrowed, "bidTime")), "{borrowed}");

    let untyped = hub.feed.create_subscription_with_types(&[EventType::Quote]).unwrap();
    let (sender, texts) = mpsc::channel();
    untyped
        .add_event_listener(move |events| {
            for event in events {
                let _ = sender.send((event.to_string(), event.to_owned().map(|e| e.to_string())));
            }
        })
        .unwrap();
    untyped.add_symbols(["IBM"]).unwrap();
    hub.publish_and_process(&[quote("IBM").into()]);
    let (borrowed, owned) = texts.try_recv().expect("no event");
    assert_eq!(Some(borrowed), owned);
}
