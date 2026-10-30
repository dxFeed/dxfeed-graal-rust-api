//! `Display` of events: the text of Java's `toString`. The expected strings are written from the Java formats.

mod support;

use std::sync::mpsc;

use dxfeed_graal::event::{Direction, Quote, Trade};
use dxfeed_graal::{Error, Event, EventRef, EventType};
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

/// Strings printed by Java 3.356 for events filled through the Java setters of the same names.
#[test]
fn typed_setters_print_as_in_java() {
    let mut quote = Quote {
        bid_exchange_code: 'Q',
        bid_price: 189.5,
        bid_size: 100.0,
        ask_exchange_code: 'N',
        ask_price: 189.75,
        ask_size: 200.0,
        time_nano_part: 456,
        ..Quote::new("AAPL")
    };
    quote.set_sequence(7).unwrap();
    assert_eq!(
        quote.to_string(),
        "Quote{AAPL, eventTime=0, time=0, timeNanoPart=456, sequence=7, bidTime=0, bidExchange=Q, bidPrice=189.5, \
         bidSize=100.0, askTime=0, askExchange=N, askPrice=189.75, askSize=200.0}"
    );

    let base = || {
        let mut trade = Trade {
            exchange_code: 'Q',
            price: 190.25,
            change: 0.5,
            size: 300.0,
            day_id: 19675,
            day_volume: 1_000_000.0,
            day_turnover: 1.9e8,
            time_nano_part: 321,
            trade_id: 42,
            ..Trade::new("AAPL")
        };
        trade.set_sequence(9).unwrap();
        trade
    };
    let prefix = "Trade{AAPL, eventTime=0, time=0, timeNanoPart=321, sequence=9, exchange=Q, tradeId=42, price=190.25, \
                  change=0.5, size=300.0, day=20231114, dayVolume=1000000.0, dayTurnover=1.9E8, ";
    let mut eth = base();
    eth.set_extended_trading_hours(true);
    assert_eq!(
        (eth.flags, eth.to_string()),
        (1, format!("{prefix}direction=UNDEFINED, ETH=true}}"))
    );
    let mut zero_down = base();
    zero_down.set_tick_direction(Direction::ZeroDown);
    assert_eq!(
        (zero_down.flags, zero_down.to_string()),
        (4, format!("{prefix}direction=ZERO_DOWN, ETH=false}}"))
    );
    let mut zero_up = base();
    zero_up.set_tick_direction(Direction::ZeroUp);
    zero_up.set_extended_trading_hours(true);
    assert_eq!(
        (zero_up.flags, zero_up.to_string()),
        (9, format!("{prefix}direction=ZERO_UP, ETH=true}}"))
    );
    assert_eq!(zero_up.tick_direction(), Direction::ZeroUp);
    assert!(zero_up.is_extended_trading_hours());
}

#[test]
fn setters_keep_the_other_packed_values() {
    let mut quote = Quote::new("AAPL");
    quote.set_sequence(5).unwrap();
    quote.set_bid_time(1_700_000_000_123);
    quote.set_ask_time(1_700_000_000_045);
    assert_eq!((quote.time(), quote.sequence()), (1_700_000_000_123, 5));
    assert_eq!(quote.time_nanos(), 1_700_000_000_123_000_000);
    assert!(matches!(quote.set_sequence(1 << 22), Err(Error::InvalidArgument(_))));
    assert!(matches!(quote.set_sequence(-1), Err(Error::InvalidArgument(_))));
    assert_eq!(quote.sequence(), 5);

    let mut trade = Trade::new("AAPL");
    trade.set_sequence(3).unwrap();
    trade.set_time_nanos(1_700_000_000_789_000_456);
    trade.set_tick_direction(Direction::Up);
    trade.set_extended_trading_hours(true);
    trade.set_tick_direction(Direction::Down);
    assert_eq!(trade.time(), 1_700_000_000_789);
    assert_eq!(trade.time_nanos(), 1_700_000_000_789_000_456);
    assert_eq!((trade.time_nano_part, trade.sequence()), (456, 3));
    assert_eq!(
        (trade.tick_direction(), trade.is_extended_trading_hours()),
        (Direction::Down, true)
    );
    trade.set_extended_trading_hours(false);
    assert_eq!(trade.flags, Direction::Down.code() << 1);
}

#[test]
fn events_convert_back_to_their_type() {
    // `quote` has no NaN, which would make the events unequal to themselves.
    let event = Event::from(quote("AAPL"));
    assert_eq!(Trade::try_from(event.clone()), Err(event.clone()));
    assert_eq!(Quote::try_from(event), Ok(quote("AAPL")));
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
