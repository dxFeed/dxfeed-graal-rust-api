//! Endpoints, subscriptions, listeners and publishing over `LOCAL_HUB` endpoints: no network needed.

mod support;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Barrier, Mutex};
use std::time::Duration;

use dxfeed_graal::event::{
    AnalyticOrder, Candle, Configuration, DailyCandle, EventFlags, Greeks, MarketMaker, Message, NuamOrder,
    NuamTimeAndSale, NuamTrade, OptionSale, Order, OrderImbalance, OtcMarketsOrder, Profile, Quote, QuoteRef, Scope,
    Series, Side, SpreadOrder, Summary, TextConfiguration, TextMessage, TheoPrice, TimeAndSale, Trade, TradeETH,
    Underlying,
};
use dxfeed_graal::{
    DXEndpoint, DXFeedSubscription, Error, Event, EventKind, EventRef, EventType, ManualExecutor, Role,
};
use support::{
    Hub, all_events, candle_symbol, collect, delivered, market_maker, order, order_base, quote, quote_with_bid,
    same_events, trade,
};

/// The quote as `LOCAL_HUB` delivers it: the SDK does not carry the event time, the millisecond-sequence and the
/// nanosecond part of a quote through the hub, they come back as zero.
fn delivered_quote(sent: Quote) -> Quote {
    Quote {
        event_time: 0,
        time_millis_sequence: 0,
        time_nano_part: 0,
        ..sent
    }
}

/// The trade as `LOCAL_HUB` delivers it: the SDK does not carry the event time and the nanosecond part of a trade
/// through the hub, they come back as zero.
fn delivered_trade(sent: Trade) -> Trade {
    Trade {
        event_time: 0,
        time_nano_part: 0,
        ..sent
    }
}

fn received(receiver: &Receiver<Event>) -> Vec<Event> {
    receiver.try_iter().collect()
}

/// Splits the orders with a special source, the price levels that the feed builds from quotes and market makers, off
/// the other events.
fn split_price_levels(events: Vec<Event>) -> (Vec<Event>, Vec<Order>) {
    let mut levels = Vec::new();
    let others = events
        .into_iter()
        .filter_map(|event| match event {
            Event::Order(order) if (1..=9).contains(&order.source()) => {
                levels.push(order);
                None
            }
            other => Some(other),
        })
        .collect();
    (others, levels)
}

#[test]
fn local_hub_builds() {
    let hub = Hub::new();
    assert_eq!(hub.endpoint.get_role(), Role::LocalHub);
    hub.executor.process_all_pending().unwrap();
}

#[test]
fn create_with_role() {
    assert_eq!(
        DXEndpoint::create_with_role(Role::Publisher).unwrap().get_role(),
        Role::Publisher
    );
}

#[test]
fn close_then_drop() {
    let hub = Hub::new();
    hub.endpoint.close().unwrap();
    hub.endpoint.close().unwrap();
}

#[test]
fn endpoint_moves_to_another_thread() {
    let endpoint = Hub::new().endpoint;
    std::thread::spawn(move || {
        assert_eq!(endpoint.get_role(), Role::LocalHub);
        drop(endpoint);
    })
    .join()
    .unwrap();
}

#[test]
fn create_makes_a_feed_endpoint() {
    let endpoint = DXEndpoint::create().unwrap();
    assert_eq!(endpoint.get_role(), Role::Feed);
}

#[test]
fn connect_chains() {
    let endpoint = DXEndpoint::create().unwrap();
    // The connection is made in the background, so an address that refuses connections is enough.
    endpoint.connect("127.0.0.1:1").unwrap().disconnect().unwrap();
}

#[test]
fn feed_and_publisher_keep_the_endpoint_open() {
    let executor = ManualExecutor::new().unwrap();
    let endpoint = DXEndpoint::new_builder()
        .with_role(Role::LocalHub)
        .with_name("chain-hub")
        .with_executor(&executor)
        .build()
        .unwrap();
    let publisher = endpoint.get_publisher();
    let feed = endpoint.get_feed();
    drop(endpoint);
    let subscription = feed.create_subscription::<Quote>().unwrap();
    drop(feed);
    let events = collect(&subscription);
    subscription.add_symbols(["AAPL"]).unwrap();
    publisher.publish_events(&[quote("AAPL").into()]).unwrap();
    executor.process_all_pending().unwrap();
    assert_eq!(received(&events), vec![Event::Quote(delivered_quote(quote("AAPL")))]);
}

#[test]
fn java_style_chain_compiles_and_works() {
    let subscription = DXEndpoint::new_builder()
        .with_role(Role::LocalHub)
        .build()
        .unwrap()
        .get_feed()
        .create_subscription::<Quote>()
        .unwrap();
    subscription.add_event_listener(|_| {}).unwrap();
    subscription.add_symbols(["AAPL"]).unwrap();
}

#[test]
fn close_and_await_termination_delivers_scheduled_events() {
    let hub = Hub::with_default_executor();
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    let events = collect(&subscription);
    subscription.add_symbols(["AAPL"]).unwrap();
    hub.publisher.publish_events(&[quote("AAPL").into()]).unwrap();
    hub.endpoint.close_and_await_termination().unwrap();
    assert_eq!(received(&events), vec![Event::Quote(delivered_quote(quote("AAPL")))]);
    hub.endpoint.close_and_await_termination().unwrap();
    hub.endpoint.close().unwrap();
}

#[test]
fn close_and_await_termination_is_refused_in_a_listener() {
    let hub = Hub::new();
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    let endpoint = hub.endpoint.clone();
    let (sender, results) = mpsc::channel();
    subscription
        .add_event_listener(move |_| {
            let _ = sender.send(endpoint.close_and_await_termination());
        })
        .unwrap();
    subscription.add_symbols(["AAPL"]).unwrap();
    hub.publish_and_process(&[quote("AAPL").into()]);
    assert!(matches!(results.try_recv(), Ok(Err(Error::InCallback))));
}

#[test]
fn symbols_from_any_iterable() {
    let hub = Hub::new();
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    let events = collect(&subscription);
    subscription.add_symbols(["AAPL", "IBM"]).unwrap();
    subscription.add_symbols(vec![String::from("MSFT")]).unwrap();
    subscription.add_symbols(std::iter::once("GOOG")).unwrap();
    subscription.remove_symbols(&["IBM"]).unwrap();
    hub.publish_and_process(&["AAPL", "IBM", "MSFT", "GOOG"].map(|s| quote(s).into()));
    let symbols: Vec<String> = received(&events)
        .into_iter()
        .map(|event| match event {
            Event::Quote(quote) => quote.symbol,
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(symbols, ["AAPL", "MSFT", "GOOG"]);
}

#[test]
fn types_are_send_and_sync() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<DXEndpoint>();
    send_sync::<dxfeed_graal::DXFeed>();
    send_sync::<dxfeed_graal::DXPublisher>();
    send_sync::<ManualExecutor>();
    send_sync::<DXFeedSubscription>();
    send_sync::<DXFeedSubscription<Quote>>();
    send_sync::<dxfeed_graal::model::IndexedTxModel<Order>>();
    send_sync::<dxfeed_graal::model::TimeSeriesTxModel<TimeAndSale>>();
    send_sync::<dxfeed_graal::model::MarketDepthModel<Order>>();
    send_sync::<dxfeed_graal::model::IndexedEventTxModel<Order>>();
}

#[test]
fn symbols_and_listeners() {
    let hub = Hub::new();
    let subscription = hub
        .feed
        .create_subscription_with_types(&[EventType::Quote, EventType::Trade])
        .unwrap();
    // Listeners go before symbols: the SDK refuses a listener for a subscription that has symbols.
    let id = subscription.add_event_listener(|_| {}).unwrap();
    subscription.add_symbols(&["AAPL", "IBM"]).unwrap();
    subscription.remove_symbols(&["IBM"]).unwrap();
    assert!(matches!(
        subscription.add_symbols(&["A\0B"]),
        Err(Error::InvalidArgument(_))
    ));
    assert!(subscription.add_event_listener(|_| {}).is_err());
    subscription.remove_event_listener(id).unwrap();
    assert!(matches!(
        subscription.remove_event_listener(id),
        Err(Error::InvalidArgument(_))
    ));
    assert!(matches!(
        hub.feed.create_subscription_with_types(&[]),
        Err(Error::InvalidArgument(_))
    ));
    subscription.close().unwrap();
}

#[test]
fn quote_round_trip() {
    let hub = Hub::new();
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    let events = collect(&subscription);
    subscription.add_symbols(&["AAPL"]).unwrap();
    hub.publish_and_process(&[quote("AAPL").into()]);
    assert_eq!(received(&events), vec![Event::Quote(delivered_quote(quote("AAPL")))]);
}

#[test]
fn trade_round_trip() {
    let hub = Hub::new();
    let subscription = hub.feed.create_subscription::<Trade>().unwrap();
    let events = collect(&subscription);
    subscription.add_symbols(&["AAPL"]).unwrap();
    hub.publish_and_process(&[trade("AAPL").into()]);
    assert_eq!(received(&events), vec![Event::Trade(delivered_trade(trade("AAPL")))]);
}

#[test]
fn typed_listener_receives_quote_refs() {
    let hub = Hub::new();
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    let (sender, quotes) = mpsc::channel();
    subscription
        .add_event_listener(move |events| {
            for quote in events {
                let quote: &QuoteRef = quote;
                let _ = sender.send(quote.to_owned());
            }
        })
        .unwrap();
    subscription.add_symbols(["AAPL"]).unwrap();
    hub.publish_and_process(&[quote("AAPL").into()]);
    assert_eq!(
        quotes.try_iter().collect::<Vec<_>>(),
        vec![delivered_quote(quote("AAPL"))]
    );
}

#[test]
fn untyped_listener_receives_event_refs_in_order() {
    let hub = Hub::new();
    let subscription = hub
        .feed
        .create_subscription_with_types(&[EventType::Quote, EventType::Trade])
        .unwrap();
    let (sender, batches) = mpsc::channel();
    subscription
        .add_event_listener(move |events| {
            let len = events.len();
            let kinds: Vec<&str> = events
                .map(|event| match event {
                    EventRef::Quote(_) => "quote",
                    EventRef::Trade(_) => "trade",
                    _ => "other",
                })
                .collect();
            let _ = sender.send((len, kinds));
        })
        .unwrap();
    subscription.add_symbols(["IBM"]).unwrap();
    hub.publish_and_process(&[quote("IBM").into(), trade("IBM").into()]);
    let kinds: Vec<&str> = batches
        .try_iter()
        .flat_map(|(len, kinds)| {
            assert_eq!(len, kinds.len());
            kinds
        })
        .collect();
    assert_eq!(kinds, ["quote", "trade"]);
}

#[test]
fn quote_subscription_receives_no_trades() {
    let hub = Hub::new();
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    subscription
        .add_event_listener(move |_| _ = c.fetch_add(1, Ordering::SeqCst))
        .unwrap();
    let events = collect(&subscription);
    subscription.add_symbols(["AAPL"]).unwrap();
    hub.publish_and_process(&[trade("AAPL").into()]);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    hub.publish_and_process(&[trade("AAPL").into(), quote("AAPL").into()]);
    assert_eq!(received(&events), vec![Event::Quote(delivered_quote(quote("AAPL")))]);
}

#[test]
fn invalid_events_are_rejected() {
    let hub = Hub::new();
    let wide = '\u{1F600}'; // Does not fit a Java char.
    let events: [Event; 3] = [
        Quote {
            bid_exchange_code: wide,
            ..quote("AAPL")
        }
        .into(),
        Trade {
            exchange_code: wide,
            ..trade("AAPL")
        }
        .into(),
        quote("A\0B").into(),
    ];
    for event in events {
        assert!(
            matches!(hub.publisher.publish_events(&[event]), Err(Error::InvalidArgument(_))),
            "an invalid event was accepted"
        );
    }
}

#[test]
fn quote_and_trade_in_order() {
    let hub = Hub::new();
    let subscription = hub
        .feed
        .create_subscription_with_types(&[EventType::Quote, EventType::Trade])
        .unwrap();
    let events = collect(&subscription);
    subscription.add_symbols(&["IBM"]).unwrap();
    hub.publish_and_process(&[quote("IBM").into(), trade("IBM").into()]);
    let got = received(&events);
    assert!(matches!(got.as_slice(), [Event::Quote(_), Event::Trade(_)]), "{got:?}");
}

#[test]
fn unsubscribed_symbol_is_not_delivered() {
    let hub = Hub::new();
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    let events = collect(&subscription);
    subscription.add_symbols(&["AAPL"]).unwrap();
    hub.publish_and_process(&[quote("MSFT").into()]);
    assert!(received(&events).is_empty());
}

#[test]
fn hubs_are_isolated() {
    // Run in parallel with the other tests that publish AAPL: each hub sees only its own events.
    for _ in 0..20 {
        let hub = Hub::new();
        let subscription = hub.feed.create_subscription::<Quote>().unwrap();
        let events = collect(&subscription);
        subscription.add_symbols(&["AAPL"]).unwrap();
        let mine = quote_with_bid("AAPL", 1.0);
        hub.publish_and_process(&[mine.clone().into()]);
        assert_eq!(received(&events), vec![Event::Quote(delivered_quote(mine))]);
    }
}

#[test]
fn panic_in_listener_keeps_it_active() {
    let hub = Hub::new();
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    subscription
        .add_event_listener(move |_| {
            if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                panic!("first batch");
            }
        })
        .unwrap();
    subscription.add_symbols(&["AAPL"]).unwrap();
    hub.publish_and_process(&[quote("AAPL").into()]);
    hub.publish_and_process(&[quote_with_bid("AAPL", 1.0).into()]);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn subscription_dropped_in_own_listener() {
    let hub = Hub::new();
    let slot: Arc<Mutex<Option<DXFeedSubscription<Quote>>>> = Arc::default();
    let calls = Arc::new(AtomicUsize::new(0));
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    let (s, c) = (slot.clone(), calls.clone());
    subscription
        .add_event_listener(move |_| {
            c.fetch_add(1, Ordering::SeqCst);
            let own = s.lock().unwrap().take();
            drop(own);
        })
        .unwrap();
    subscription.add_symbols(&["AAPL"]).unwrap();
    *slot.lock().unwrap() = Some(subscription);
    hub.publish_and_process(&[quote("AAPL").into()]);
    hub.publish_and_process(&[quote_with_bid("AAPL", 1.0).into()]);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(slot.lock().unwrap().is_none());
}

#[test]
fn remove_listener_waits_for_running_call() {
    let hub = Arc::new(Hub::new());
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    let started = Arc::new(Barrier::new(2));
    let finished = Arc::new(AtomicBool::new(false));
    let (b, f) = (started.clone(), finished.clone());
    let id = subscription
        .add_event_listener(move |_| {
            b.wait();
            std::thread::sleep(Duration::from_millis(300));
            f.store(true, Ordering::SeqCst);
        })
        .unwrap();
    subscription.add_symbols(&["AAPL"]).unwrap();
    hub.publisher.publish_events(&[quote("AAPL").into()]).unwrap();
    let processor = {
        let hub = hub.clone();
        std::thread::spawn(move || hub.executor.process_all_pending().unwrap())
    };
    started.wait();
    subscription.remove_event_listener(id).unwrap();
    assert!(
        finished.load(Ordering::SeqCst),
        "remove_event_listener returned while the call was running"
    );
    processor.join().unwrap();
}

#[test]
fn concurrent_processing_delivers_each_event_once() {
    let hub = Arc::new(Hub::new());
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let c = count.clone();
    subscription
        .add_event_listener(move |events| _ = c.fetch_add(events.count(), Ordering::SeqCst))
        .unwrap();
    let symbols: Vec<String> = (0..200).map(|i| format!("S{i}")).collect();
    subscription.add_symbols(&symbols).unwrap();
    let events: Vec<Event> = symbols.iter().map(|s| quote(s).into()).collect();
    for chunk in events.chunks(10) {
        hub.publisher.publish_events(chunk).unwrap();
    }
    let threads: Vec<_> = (0..2)
        .map(|_| {
            let hub = hub.clone();
            std::thread::spawn(move || hub.executor.process_all_pending().unwrap())
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    hub.executor.process_all_pending().unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 200);
}

#[test]
fn processing_from_listener_does_not_nest_calls() {
    let hub = Arc::new(Hub::new());
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    let (calls, inside, nested) = (
        Arc::new(AtomicUsize::new(0)),
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    );
    let (h, c, i, n) = (Arc::downgrade(&hub), calls.clone(), inside.clone(), nested.clone());
    subscription
        .add_event_listener(move |_| {
            if i.swap(true, Ordering::SeqCst) {
                n.store(true, Ordering::SeqCst);
            }
            if c.fetch_add(1, Ordering::SeqCst) == 0
                && let Some(hub) = h.upgrade()
            {
                // The SDK serializes the calls of a subscription: this quote comes in a second call after this one.
                hub.publish_and_process(&[quote_with_bid("AAPL", 1.0).into()]);
            }
            i.store(false, Ordering::SeqCst);
        })
        .unwrap();
    subscription.add_symbols(&["AAPL"]).unwrap();
    hub.publish_and_process(&[quote("AAPL").into()]);
    hub.executor.process_all_pending().unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(!nested.load(Ordering::SeqCst));
}

#[test]
fn listeners_drop_each_other_without_deadlock() {
    let hub = Hub::with_default_executor();
    let slots: [Arc<Mutex<Option<DXFeedSubscription<Quote>>>>; 2] = [Arc::default(), Arc::default()];
    let started: Arc<[AtomicBool; 2]> = Arc::default();
    let (done, wait) = mpsc::channel();
    for i in 0..2 {
        let subscription = hub.feed.create_subscription::<Quote>().unwrap();
        let (other, started, done) = (slots[1 - i].clone(), started.clone(), done.clone());
        subscription
            .add_event_listener(move |_| {
                // Wait a little for the other listener, so both drop each other while both run, if the executor
                // runs them on two threads.
                started[i].store(true, Ordering::SeqCst);
                let until = std::time::Instant::now() + Duration::from_secs(1);
                while !started[1 - i].load(Ordering::SeqCst) && std::time::Instant::now() < until {
                    std::thread::yield_now();
                }
                let taken = other.lock().unwrap().take();
                drop(taken);
                let _ = done.send(i);
            })
            .unwrap();
        subscription.add_symbols(&["AAPL"]).unwrap();
        *slots[i].lock().unwrap() = Some(subscription);
    }
    hub.publisher.publish_events(&[quote("AAPL").into()]).unwrap();
    wait.recv_timeout(Duration::from_secs(10)).expect("no listener ran");
    let _ = wait.recv_timeout(Duration::from_secs(2));
    // Whatever the listeners left is dropped here; neither must hang.
    let (cleaned, cleanup) = mpsc::channel();
    let remaining = slots.clone();
    std::thread::spawn(move || {
        for slot in remaining {
            let taken = slot.lock().unwrap().take();
            drop(taken);
        }
        cleaned.send(()).unwrap();
    });
    cleanup.recv_timeout(Duration::from_secs(10)).expect("deadlock");
    while wait.recv_timeout(Duration::from_millis(500)).is_ok() {}
    hub.publisher
        .publish_events(&[quote_with_bid("AAPL", 2.0).into()])
        .unwrap();
    assert!(
        wait.recv_timeout(Duration::from_millis(500)).is_err(),
        "a dropped subscription still receives"
    );
}

#[test]
fn hub_with_pending_tasks_drops_quickly() {
    let hub = Hub::new();
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    subscription.add_event_listener(|_| {}).unwrap();
    subscription.add_symbols(&["AAPL"]).unwrap();
    hub.publisher.publish_events(&[quote("AAPL").into()]).unwrap();
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        drop(subscription);
        drop(hub);
        sender.send(()).unwrap();
    });
    receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("dropping the hub took over 5 seconds");
}

/// Every mapped type, in the order of [`all_events`].
const ALL_TYPES: [EventType; 26] = [
    EventType::Quote,
    EventType::Profile,
    EventType::Summary,
    EventType::Greeks,
    EventType::Underlying,
    EventType::TheoPrice,
    EventType::Trade,
    EventType::TradeETH,
    EventType::Series,
    EventType::NuamTrade,
    EventType::TimeAndSale,
    EventType::NuamTimeAndSale,
    EventType::OptionSale,
    EventType::Candle,
    EventType::DailyCandle,
    EventType::Order,
    EventType::AnalyticOrder,
    EventType::OtcMarketsOrder,
    EventType::SpreadOrder,
    EventType::NuamOrder,
    EventType::MarketMaker,
    EventType::OrderImbalance,
    EventType::Configuration,
    EventType::Message,
    EventType::TextMessage,
    EventType::TextConfiguration,
];

#[test]
fn every_type_round_trips() {
    let hub = Hub::new();
    let subscription = hub.feed.create_subscription_with_types(&ALL_TYPES).unwrap();
    let events = collect(&subscription);
    subscription
        .add_symbols(["AAPL".to_owned(), candle_symbol("AAPL")])
        .unwrap();
    hub.publish_and_process(&all_events("AAPL"));
    // The hub keeps the order within a type only: a series may come after a later Nuam trade. The candles arrive
    // with the symbol they were sent with; the quote and the market maker also arrive as two price levels each.
    let (received, levels) = split_price_levels(received(&events));
    assert_eq!(levels.len(), 4, "{levels:?}");
    assert_eq!(received.len(), ALL_TYPES.len(), "{received:?}");
    for expected in all_events("AAPL").into_iter().map(delivered) {
        assert!(
            received.iter().any(|e| same_events(e, &expected)),
            "{expected:?} not in {received:?}"
        );
    }
}

/// Subscribes to `E` only, publishes one event of each type and returns what the listener received.
fn receive_typed<E>() -> Vec<Event>
where
    E: EventKind,
    for<'a> E::Item<'a>: support::ToEvent,
{
    let hub = Hub::new();
    let subscription = hub.feed.create_subscription::<E>().unwrap();
    let events = collect(&subscription);
    subscription
        .add_symbols(["AAPL".to_owned(), candle_symbol("AAPL")])
        .unwrap();
    hub.publish_and_process(&all_events("AAPL"));
    received(&events)
}

#[test]
fn typed_subscriptions_receive_only_their_type() {
    let all = all_events("AAPL");
    let check = |received: Vec<Event>, index: usize| {
        let expected = [delivered(all[index].clone())];
        assert!(
            same_events(&received[..], &expected[..]),
            "{:?}: {received:?}",
            ALL_TYPES[index]
        );
    };
    check(receive_typed::<Quote>(), 0);
    check(receive_typed::<Profile>(), 1);
    check(receive_typed::<Summary>(), 2);
    check(receive_typed::<Greeks>(), 3);
    check(receive_typed::<Underlying>(), 4);
    check(receive_typed::<TheoPrice>(), 5);
    check(receive_typed::<Trade>(), 6);
    check(receive_typed::<TradeETH>(), 7);
    check(receive_typed::<Series>(), 8);
    check(receive_typed::<NuamTrade>(), 9);
    check(receive_typed::<TimeAndSale>(), 10);
    check(receive_typed::<NuamTimeAndSale>(), 11);
    check(receive_typed::<OptionSale>(), 12);
    check(receive_typed::<Candle>(), 13);
    check(receive_typed::<DailyCandle>(), 14);
    let (orders, levels) = split_price_levels(receive_typed::<Order>());
    assert_eq!(levels.len(), 4, "{levels:?}");
    check(orders, 15);
    check(receive_typed::<AnalyticOrder>(), 16);
    check(receive_typed::<OtcMarketsOrder>(), 17);
    check(receive_typed::<SpreadOrder>(), 18);
    check(receive_typed::<NuamOrder>(), 19);
    check(receive_typed::<MarketMaker>(), 20);
    check(receive_typed::<OrderImbalance>(), 21);
    check(receive_typed::<Configuration>(), 22);
    check(receive_typed::<Message>(), 23);
    check(receive_typed::<TextMessage>(), 24);
    check(receive_typed::<TextConfiguration>(), 25);
}

/// An attachment arrives as it was sent: none, empty or text.
#[test]
fn attachments_survive_the_hub() {
    let hub = Hub::new();
    let subscription = hub.feed.create_subscription::<Message>().unwrap();
    let events = collect(&subscription);
    subscription.add_symbols(["AAPL"]).unwrap();
    let sent: Vec<Event> = [None, Some(""), Some("x")]
        .into_iter()
        .map(|attachment| {
            Message {
                attachment: attachment.map(str::to_owned),
                ..Message::new("AAPL")
            }
            .into()
        })
        .collect();
    hub.publish_and_process(&sent);
    assert_eq!(received(&events), sent);
}

#[test]
fn attachment_with_nul_is_rejected() {
    let hub = Hub::new();
    let message = Message {
        attachment: Some("a\0b".to_owned()),
        ..Message::new("AAPL")
    };
    let result = hub.publisher.publish_events(&[message.into()]);
    assert!(matches!(result, Err(Error::InvalidArgument(_))), "{result:?}");
}

/// Java refuses these orders when they are published, with an `IllegalArgumentException` or, for an order base, a
/// `NullPointerException`.
#[test]
fn invalid_orders_are_refused_by_java() {
    let hub = Hub::new();
    let mut undefined_side = order("AAPL");
    undefined_side.set_order_side(Side::Undefined);
    let events: [Event; 5] = [
        order_base("AAPL").into(),
        Order {
            index: -1,
            ..order("AAPL")
        }
        .into(),
        Order {
            event_flags: EventFlags::SNAPSHOT_END,
            ..order("AAPL")
        }
        .into(),
        undefined_side.into(),
        Order {
            market_maker: Some("ABCDEFGHI".to_owned()),
            ..order("AAPL")
        }
        .into(),
    ];
    for event in events {
        let result = hub.publisher.publish_events(std::slice::from_ref(&event));
        assert!(matches!(result, Err(Error::Java(_))), "{event}: {result:?}");
    }
}

/// The hub keeps 4 characters of the market maker of an order: one of 5 to 8 characters loses the first ones.
#[test]
fn order_market_maker_keeps_4_characters() {
    let hub = Hub::new();
    let subscription = hub.feed.create_subscription::<Order>().unwrap();
    let events = collect(&subscription);
    subscription.add_symbols(["AAPL"]).unwrap();
    let long = Order {
        market_maker: Some("MM123".to_owned()),
        ..order("AAPL")
    };
    hub.publish_and_process(&[long.into()]);
    let received = received(&events);
    assert!(
        matches!(&received[..], [Event::Order(o)] if o.market_maker.as_deref() == Some("M123")),
        "{received:?}"
    );
}

/// The feed builds orders from quotes and market makers too, as the dxFeed scheme maps the `Quote` and `MarketMaker`
/// records: the price levels of the composite and the aggregate books, with special sources.
#[test]
fn quotes_and_market_makers_arrive_as_price_levels() {
    let hub = Hub::new();
    let subscription = hub.feed.create_subscription::<Order>().unwrap();
    let events = collect(&subscription);
    subscription.add_symbols(["AAPL"]).unwrap();
    hub.publish_and_process(&[quote("AAPL").into(), market_maker("AAPL").into()]);
    let (orders, mut levels) = split_price_levels(received(&events));
    assert!(orders.is_empty(), "{orders:?}");
    levels.sort_by_key(Order::source);
    let levels: Vec<_> = levels
        .iter()
        .map(|o| {
            (
                o.source(),
                o.scope(),
                o.order_side(),
                o.exchange_code(),
                o.price,
                o.size,
                o.market_maker.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        levels,
        [
            (1, Scope::Composite, Side::Buy, 'Q', 189.5, 100.0, None),
            (2, Scope::Composite, Side::Sell, 'N', 189.75, 200.0, None),
            (5, Scope::Aggregate, Side::Buy, 'Q', 189.5, 100.0, Some("MM5")),
            (6, Scope::Aggregate, Side::Sell, 'Q', 189.75, 200.0, Some("MM5")),
        ]
    );
}
