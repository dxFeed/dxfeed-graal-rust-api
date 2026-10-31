//! The last-event lookups of a feed over `LOCAL_HUB` endpoints: no network needed.

mod support;

use std::fmt::Debug;
use std::sync::mpsc;
use std::sync::{Mutex, MutexGuard, PoisonError};

use dxfeed_graal::event::{
    Candle, Configuration, DailyCandle, Greeks, LastingEvent, NuamTrade, Profile, Quote, Summary, TextConfiguration,
    TheoPrice, Trade, TradeETH, Underlying,
};
use dxfeed_graal::{Error, Event, EventKind, live_events};
use support::{
    Hub, candle, candle_symbol, configuration, daily_candle, delivered, greeks, nuam_trade, profile, quote,
    quote_with_bid, same_events, summary, text_configuration, theo_price, trade, trade_eth, underlying,
};

/// Runs the tests of this file one by one, so that each can check that no SDK event is left unreleased.
fn serial() -> MutexGuard<'static, ()> {
    static SERIAL: Mutex<()> = Mutex::new(());
    SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Looks up the last event of `sample`'s type and symbol before a subscription, before data and after publishing.
fn check_lookups<E: LastingEvent + EventKind + Debug>(new: E, sample: E, symbol: &str) {
    let hub = Hub::new();
    assert!(
        hub.feed.get_last_event_if_subscribed::<E>(symbol).unwrap().is_none(),
        "{new:?}"
    );

    let subscription = hub.feed.create_subscription::<E>().unwrap();
    subscription.add_symbols([symbol]).unwrap();
    let empty = hub.feed.get_last_event_if_subscribed::<E>(symbol).unwrap().unwrap();
    assert!(same_events(&empty, &new), "{empty:?}");

    hub.publish_and_process(&[sample.clone().into()]);
    let expected = delivered(sample.into());
    let last: Event = hub
        .feed
        .get_last_event_if_subscribed::<E>(symbol)
        .unwrap()
        .unwrap()
        .into();
    assert!(same_events(&last, &expected), "{last:?}");
    let last: Event = hub.feed.get_last_event(&new).unwrap().into();
    assert!(same_events(&last, &expected), "{last:?}");
}

#[test]
fn every_lasting_type_has_a_last_event() {
    let _serial = serial();
    let s = "AAPL";
    let c = &candle_symbol(s);
    check_lookups(Quote::new(s), quote(s), s);
    check_lookups(Profile::new(s), profile(s), s);
    check_lookups(Summary::new(s), summary(s), s);
    check_lookups(Greeks::new(s), greeks(s), s);
    check_lookups(Underlying::new(s), underlying(s), s);
    check_lookups(TheoPrice::new(s), theo_price(s), s);
    check_lookups(Trade::new(s), trade(s), s);
    check_lookups(TradeETH::new(s), trade_eth(s), s);
    check_lookups(NuamTrade::new(s), nuam_trade(s), s);
    check_lookups(Candle::new(c), candle(c), c);
    check_lookups(DailyCandle::new(c), daily_candle(c), c);
    check_lookups(Configuration::new(s), configuration(s), s);
    check_lookups(TextConfiguration::new(s), text_configuration(s), s);
    assert_eq!(live_events(), 0);
}

/// A subscription to another type, to another symbol or on another feed does not count.
#[test]
fn other_subscriptions_do_not_count() {
    let _serial = serial();
    let hub = Hub::new();
    let other = Hub::new();
    let trades = hub.feed.create_subscription::<Trade>().unwrap();
    trades.add_symbols(["AAPL"]).unwrap();
    let quotes = other.feed.create_subscription::<Quote>().unwrap();
    quotes.add_symbols(["AAPL"]).unwrap();
    let ibm = hub.feed.create_subscription::<Quote>().unwrap();
    ibm.add_symbols(["IBM"]).unwrap();
    assert!(
        hub.feed
            .get_last_event_if_subscribed::<Quote>("AAPL")
            .unwrap()
            .is_none()
    );
    assert_eq!(live_events(), 0);
}

/// Without a subscription the input comes back as it was, with every field.
#[test]
fn last_event_without_data_is_the_input() {
    let _serial = serial();
    let hub = Hub::new();
    // Java recomputes the milliseconds of the packed sequence from the later of the bid and ask times, 0 here.
    let sample = Quote {
        time_millis_sequence: 7,
        ..quote("AAPL")
    };
    assert_eq!(hub.feed.get_last_event(&sample).unwrap(), sample);
    assert_eq!(live_events(), 0);
}

/// A listener sees the last event at least as new as the one it receives: the cache is updated on publishing.
#[test]
fn last_event_inside_a_listener() {
    let _serial = serial();
    let hub = Hub::new();
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    let feed = hub.feed.clone();
    let (sender, receiver) = mpsc::channel();
    subscription
        .add_event_listener(move |quotes| {
            for quote in quotes {
                let last = feed.get_last_event_if_subscribed::<Quote>(&quote.symbol()).unwrap();
                let _ = sender.send((quote.bid_price(), last.map(|q| q.bid_price)));
            }
        })
        .unwrap();
    subscription.add_symbols(["AAPL"]).unwrap();
    hub.publish_and_process(&[quote_with_bid("AAPL", 10.0).into()]);
    hub.publish_and_process(&[quote_with_bid("AAPL", 11.0).into()]);
    let seen: Vec<_> = receiver.try_iter().collect();
    assert_eq!(seen, [(10.0, Some(10.0)), (11.0, Some(11.0))]);
    assert_eq!(live_events(), 0);
}

#[test]
fn invalid_symbols_are_rejected() {
    let _serial = serial();
    let hub = Hub::new();
    let nul = hub.feed.get_last_event_if_subscribed::<Quote>("A\0B");
    assert!(matches!(nul, Err(Error::InvalidArgument(_))), "{nul:?}");
    let candle = hub.feed.get_last_event_if_subscribed::<Candle>("bad{=zzz}");
    assert!(matches!(candle, Err(Error::Java(_))), "{candle:?}");
    let input = hub.feed.get_last_event(&Quote::new("A\0B"));
    assert!(matches!(input, Err(Error::InvalidArgument(_))), "{input:?}");
    assert_eq!(live_events(), 0);
}
