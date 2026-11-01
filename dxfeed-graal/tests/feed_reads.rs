//! The bulk and cached reads of a feed and the symbol kinds its requests take.

mod support;

use std::collections::HashSet;
use std::time::Duration;

use dxfeed_graal::event::{BaseEvent, Candle, CandleSymbol, IndexedEventSource, Order, Quote, Series, TimeAndSale};
use dxfeed_graal::{Error, Promises, Symbol};
use support::{Hub, candle, candle_symbol, order, quote_with_bid, series, time_and_sale};

const LONG: Duration = Duration::from_secs(10);

#[test]
fn get_last_events_fills_several_quotes_at_once() {
    let hub = Hub::new();
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    subscription.add_symbols(["AAPL", "IBM", "MSFT"]).unwrap();
    hub.publish_and_process(&[
        quote_with_bid("AAPL", 1.0).into(),
        quote_with_bid("IBM", 2.0).into(),
        quote_with_bid("MSFT", 3.0).into(),
    ]);
    let asked = ["MSFT", "AAPL", "IBM", "GOOG"].map(Quote::new);
    let last = hub.feed.get_last_events(&asked).unwrap();
    let symbols: Vec<_> = last.iter().map(|quote| quote.symbol().into_owned()).collect();
    assert_eq!(symbols, ["MSFT", "AAPL", "IBM", "GOOG"]);
    assert_eq!(
        last[..3].iter().map(|quote| quote.bid_price).collect::<Vec<_>>(),
        [3.0, 1.0, 2.0]
    );
    assert!(last[3].bid_price.is_nan(), "an unknown quote is returned as given");
    assert!(hub.feed.get_last_events::<Quote>(&[]).unwrap().is_empty());

    let promises = hub.feed.get_last_events_promises::<Quote>(["AAPL", "IBM"]).unwrap();
    assert_eq!(promises.len(), 2);
    Promises::all_of(&promises).unwrap().wait(LONG).unwrap();
    assert_eq!(promises[1].wait(Duration::ZERO).unwrap().bid_price, 2.0);
}

#[test]
fn cached_series_need_a_subscription() {
    let hub = Hub::new();
    let read = || {
        hub.feed
            .get_time_series_if_subscribed::<TimeAndSale>("AAPL", 0, i64::MAX)
            .unwrap()
    };
    assert!(read().is_empty(), "no subscription");
    let sales = hub.feed.create_time_series_subscription::<TimeAndSale>().unwrap();
    sales.set_from_time(0).unwrap();
    sales.add_symbols(["AAPL"]).unwrap();
    hub.publish_and_process(&[time_and_sale("AAPL").into()]);
    assert_eq!(read().len(), 1);
    sales.close().unwrap();
    assert!(read().is_empty(), "the subscription is closed");

    let read = || {
        hub.feed
            .get_indexed_events_if_subscribed::<Series>("AAPL", IndexedEventSource::DEFAULT)
            .unwrap()
    };
    assert!(read().is_empty(), "no subscription");
    let subscription = hub.feed.create_subscription::<Series>().unwrap();
    subscription.add_symbols(["AAPL"]).unwrap();
    hub.publish_and_process(&[series("AAPL").into()]);
    assert_eq!(read().len(), 1);
}

#[test]
fn cached_orders_are_read_by_source() {
    let hub = Hub::new();
    let read = |source| hub.feed.get_indexed_events_if_subscribed::<Order>("AAPL", source);
    let subscription = hub.feed.create_subscription::<Order>().unwrap();
    subscription
        .add_symbols([Symbol::indexed("AAPL", IndexedEventSource::NTV)])
        .unwrap();
    let mut ntv = order("AAPL");
    ntv.set_source(IndexedEventSource::NTV).unwrap();
    hub.publish_and_process(&[ntv.into()]);
    let orders = read(IndexedEventSource::NTV).unwrap();
    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0].source(), IndexedEventSource::NTV);
    assert!(read(IndexedEventSource::NFX).unwrap().is_empty(), "another source");
    let unknown = IndexedEventSource::new(1000, "UNKNOWN");
    assert!(matches!(read(unknown), Err(Error::InvalidArgument(_))));
}

#[test]
fn requests_take_candle_symbols() {
    let hub = Hub::new();
    let text = candle_symbol("AAPL");
    let symbol: CandleSymbol = text.parse().unwrap();
    let subscription = hub.feed.create_subscription::<Candle>().unwrap();
    subscription.add_symbols([&symbol]).unwrap();
    hub.publish_and_process(&[candle(&text).into()]);
    let last = hub
        .feed
        .get_last_event_promise::<Candle>(&symbol)
        .unwrap()
        .wait(LONG)
        .unwrap();
    assert_eq!(last.symbol(), text);
    assert!(
        hub.feed
            .get_last_event_if_subscribed::<Candle>(symbol)
            .unwrap()
            .is_some()
    );
    assert!(
        hub.feed
            .get_last_event_if_subscribed::<Candle>(text.as_str())
            .unwrap()
            .is_some()
    );

    let last = hub.feed.get_last_events(&[Candle::new(&text)]).unwrap();
    assert_eq!(last[0].symbol(), text);
    let promises = hub.feed.get_last_events_promises::<Candle>([text.as_str()]).unwrap();
    assert_eq!(promises[0].wait(LONG).unwrap().symbol(), text);
}

#[test]
fn symbols_have_accessors_and_hash() {
    let indexed = Symbol::indexed("AAPL", IndexedEventSource::DEFAULT);
    let time_series = Symbol::time_series("AAPL", 1000);
    let plain = Symbol::from("AAPL");
    assert_eq!(indexed.get_event_symbol(), &plain);
    assert_eq!(time_series.get_event_symbol(), &plain);
    assert_eq!(plain.get_event_symbol(), &plain);
    assert_eq!(indexed.get_source(), Some(IndexedEventSource::DEFAULT));
    assert_eq!(time_series.get_source(), None);
    assert_eq!(time_series.get_from_time(), Some(1000));
    assert_eq!(indexed.get_from_time(), None);
    assert_eq!(Symbol::WILDCARD_RESERVED_PREFIX, Symbol::Wildcard.to_string());
    let set: HashSet<Symbol> = [plain.clone(), plain, indexed, time_series].into_iter().collect();
    assert_eq!(set.len(), 3);
}
