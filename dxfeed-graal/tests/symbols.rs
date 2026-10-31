//! Symbols of every kind, time-series subscriptions, list promises, candle symbols and time formats; over `LOCAL_HUB`
//! endpoints, no network needed.

mod support;

use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use dxfeed_graal::event::{
    Candle, CandleAlignment, CandleAttribute, CandlePeriod, CandlePrice, CandleSession, CandleSymbol, CandleType,
    EventFlags, IndexedEvent, Order, OrderSource, Quote, TimeSeriesEvent,
};
use dxfeed_graal::{DXEndpoint, Error, Event, Role, Symbol, TimeFormat, live_events};
use support::{Hub, candle, collect, order, quote};

/// Serializes the tests that count the live events, which are process-wide.
fn counting_events() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

const DAY: i64 = 86_400_000;
const FROM: i64 = 1_700_000_000_000;

/// A daily candle of `symbol` at `time`.
fn candle_at(symbol: &str, time: i64) -> Event {
    flagged_candle(symbol, time, EventFlags::default())
}

/// A daily candle of `symbol` at `time` with `flags`.
fn flagged_candle(symbol: &str, time: i64, flags: EventFlags) -> Event {
    let mut candle = candle(symbol);
    candle.set_time(time);
    candle.event_flags = flags;
    candle.into()
}

/// An order of `AAPL` from `source` with the index `index` and `flags`.
fn flagged_order(source: OrderSource, index: i64, flags: EventFlags) -> Event {
    let mut order = order("AAPL");
    order.index = index;
    order.set_source(source.get_id()).unwrap();
    order.event_flags = flags;
    order.into()
}

fn candle_times(events: &[Event]) -> Vec<i64> {
    events
        .iter()
        .map(|event| match event {
            Event::Candle(candle) => candle.time(),
            other => panic!("not a candle: {other:?}"),
        })
        .collect()
}

#[test]
fn candle_symbol_round_trips() {
    let hub = Hub::new();
    let subscription = hub.feed.create_subscription::<Candle>().unwrap();
    let events = collect(&subscription);
    let symbol = CandleSymbol::value_of("AAPL{=1d}").unwrap();
    subscription.add_symbols([&symbol]).unwrap();
    hub.publish_and_process(&[candle_at("AAPL{=d}", FROM)]);
    let received: Vec<Event> = events.try_iter().collect();
    assert_eq!(candle_times(&received), [FROM]);
    subscription.remove_symbols([symbol]).unwrap();
    hub.publish_and_process(&[candle_at("AAPL{=d}", FROM + DAY)]);
    assert_eq!(events.try_iter().count(), 0);
}

#[test]
fn wildcard_receives_every_symbol() {
    let endpoint = DXEndpoint::new_builder()
        .with_role(Role::LocalHub)
        .with_property("dxfeed.wildcard.enable", "true")
        .build()
        .unwrap();
    let subscription = endpoint.get_feed().create_subscription::<Quote>().unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    subscription
        .add_event_listener(move |quotes| {
            for quote in quotes {
                let _ = sender.send(quote.symbol().into_owned());
            }
        })
        .unwrap();
    subscription.add_symbols([Symbol::Wildcard]).unwrap();
    endpoint
        .get_publisher()
        .publish_events(&[quote("IBM").into(), quote("MSFT").into()])
        .unwrap();
    let mut symbols: Vec<String> = (0..2)
        .map(|_| receiver.recv_timeout(Duration::from_secs(10)).unwrap())
        .collect();
    symbols.sort();
    assert_eq!(symbols, ["IBM", "MSFT"]);
}

#[test]
fn time_series_symbol_starts_at_its_time() {
    let hub = Hub::new();
    let subscription = hub.feed.create_subscription::<Candle>().unwrap();
    let events = collect(&subscription);
    subscription
        .add_symbols([Symbol::time_series(CandleSymbol::value_of("AAPL{=d}").unwrap(), FROM)])
        .unwrap();
    hub.publish_and_process(&[
        candle_at("AAPL{=d}", FROM + DAY),
        candle_at("AAPL{=d}", FROM),
        candle_at("AAPL{=d}", FROM - DAY),
    ]);
    let received: Vec<Event> = events.try_iter().collect();
    assert_eq!(candle_times(&received), [FROM + DAY, FROM]);
}

#[test]
fn time_series_subscription_starts_at_its_time() {
    let hub = Hub::new();
    let subscription = hub.feed.create_time_series_subscription::<Candle>().unwrap();
    let events = collect(&subscription);
    subscription.set_from_time(FROM).unwrap();
    subscription.add_symbols(["AAPL{=d}"]).unwrap();
    hub.publish_and_process(&[candle_at("AAPL{=d}", FROM + DAY), candle_at("AAPL{=d}", FROM - DAY)]);
    let received: Vec<Event> = events.try_iter().collect();
    assert_eq!(candle_times(&received), [FROM + DAY]);
}

#[test]
fn indexed_symbol_selects_its_source() {
    let hub = Hub::new();
    let subscription = hub.feed.create_subscription::<Order>().unwrap();
    let events = collect(&subscription);
    subscription
        .add_symbols([Symbol::indexed("AAPL", OrderSource::NTV)])
        .unwrap();
    let flags = EventFlags::default();
    hub.publish_and_process(&[
        flagged_order(OrderSource::NTV, 1, flags),
        flagged_order(OrderSource::DEA, 1, flags),
    ]);
    let sources: Vec<i32> = events
        .try_iter()
        .map(|event| match event {
            Event::Order(order) => order.source(),
            other => panic!("not an order: {other:?}"),
        })
        .collect();
    assert_eq!(sources, [OrderSource::NTV.get_id()]);
}

#[test]
fn list_promises_complete_with_a_snapshot() {
    let _counting = counting_events();
    let hub = Hub::with_default_executor();
    // A promise completes with the snapshot the uplink sends for its subscription: here the publisher of the hub.
    let candles = hub
        .feed
        .get_time_series_promise::<Candle>("AAPL{=d}", FROM - DAY, FROM + DAY)
        .unwrap();
    let orders = hub
        .feed
        .get_indexed_events_promise::<Order>("AAPL", OrderSource::NTV)
        .unwrap();
    hub.publisher
        .publish_events(&[
            flagged_candle("AAPL{=d}", FROM, EventFlags::SNAPSHOT_BEGIN),
            flagged_candle("AAPL{=d}", FROM - DAY, EventFlags::default()),
            flagged_candle("AAPL{=d}", 0, EventFlags::SNAPSHOT_END | EventFlags::REMOVE_EVENT),
            flagged_order(OrderSource::NTV, 2, EventFlags::SNAPSHOT_BEGIN),
            flagged_order(OrderSource::NTV, 1, EventFlags::default()),
            // An indexed snapshot ends with index 0; removed, it is no event of the snapshot.
            flagged_order(OrderSource::NTV, 0, EventFlags::SNAPSHOT_END | EventFlags::REMOVE_EVENT),
        ])
        .unwrap();
    // Both lists come ordered by index: the candles by time.
    let candles = candles.wait(Duration::from_secs(10)).unwrap();
    let times: Vec<i64> = candles.iter().map(TimeSeriesEvent::time).collect();
    assert_eq!(times, [FROM - DAY, FROM]);
    let orders = orders.wait(Duration::from_secs(10)).unwrap();
    let orders: Vec<(i32, i64)> = orders
        .iter()
        .map(|order| (order.source(), order.index() & 0xffff_ffff))
        .collect();
    assert_eq!(orders, [(OrderSource::NTV.get_id(), 1), (OrderSource::NTV.get_id(), 2)]);
    assert_eq!(live_events(), 0);
}

#[test]
fn cancelled_list_promises_fail() {
    let _counting = counting_events();
    let hub = Hub::with_default_executor();
    let candles = hub
        .feed
        .get_time_series_promise::<Candle>("AAPL{=d}", FROM - DAY, FROM + DAY)
        .unwrap();
    candles.cancel().unwrap();
    // A list promise without a result fails rather than gives an empty list.
    assert!(matches!(candles.wait(Duration::from_secs(10)), Err(Error::Java(_))));
    assert!(matches!(candles.get_result(), Err(Error::Java(_))));
    assert_eq!(live_events(), 0);
}

#[test]
fn candle_symbols_are_normalized() {
    assert_eq!(CandleSymbol::value_of("AAPL{=1d}").unwrap().as_str(), "AAPL{=d}");
    assert_eq!(
        CandleSymbol::value_of_with("AAPL", [CandlePeriod::DAY])
            .unwrap()
            .to_string(),
        "AAPL{=d}"
    );
    let symbol = CandleSymbol::value_of_with(
        "AAPL{=d}",
        [
            CandleAttribute::Exchange('Q'),
            CandlePeriod::value_of(5.0, CandleType::Minute).into(),
            CandlePrice::Bid.into(),
            CandleSession::Regular.into(),
            CandleAlignment::Session.into(),
            CandleAttribute::PriceLevel(0.5),
        ],
    )
    .unwrap();
    assert_eq!(symbol.as_str(), "AAPL&Q{=5m,a=s,pl=0.5,price=bid,tho=true}");
    assert_eq!(
        CandleSymbol::value_of_with("AAPL{=d}", [CandlePrice::Last])
            .unwrap()
            .as_str(),
        "AAPL{=d}"
    );
    assert!(matches!(CandleSymbol::value_of("AAPL{=zzz}"), Err(Error::Java(_))));
}

#[test]
fn time_formats_parse_and_format() {
    let time = 1_167_654_896_000;
    assert_eq!(TimeFormat::GMT.parse("20070101-123456").unwrap(), time);
    assert_eq!(TimeFormat::GMT.parse("2007-01-01 15:34:56+03:00").unwrap(), time);
    assert_eq!(TimeFormat::GMT.parse("1167654896000").unwrap(), time);
    assert_eq!(TimeFormat::GMT.format(time).unwrap(), "20070101-123456");
    assert_eq!(
        TimeFormat::GMT.with_millis().format(time + 7).unwrap(),
        "20070101-123456.007"
    );
    for format in [
        TimeFormat::DEFAULT,
        TimeFormat::GMT.with_time_zone(),
        TimeFormat::DEFAULT.with_millis().with_time_zone(),
        TimeFormat::GMT.as_full_iso(),
    ] {
        let text = format.format(time).unwrap();
        assert_eq!(format.parse(&text).unwrap(), time, "{format:?}: {text}");
    }
    assert!(matches!(TimeFormat::GMT.parse("not a time"), Err(Error::Java(_))));
}

#[test]
fn symbols_print_as_in_java() {
    assert_eq!(Symbol::from("AAPL").to_string(), "AAPL");
    assert_eq!(Symbol::Wildcard.to_string(), "*");
    assert_eq!(
        Symbol::indexed("AAPL", OrderSource::NTV).to_string(),
        "AAPL{source=NTV}"
    );
    let text = TimeFormat::DEFAULT.with_millis().format(FROM).unwrap();
    assert_eq!(
        Symbol::time_series("AAPL", FROM).to_string(),
        format!("AAPL{{fromTime={text}}}")
    );
}
