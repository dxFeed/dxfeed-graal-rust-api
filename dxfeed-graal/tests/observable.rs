//! Observable subscriptions: the change listeners of the subscriptions of a publisher and of a feed; over `LOCAL_HUB`
//! endpoints, no network needed.

mod support;

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use dxfeed_graal::event::{Candle, CandleSymbol, Order, OrderSource, Profile};
use dxfeed_graal::{
    DXEndpoint, Error, Event, EventType, ListenerId, ObservableSubscription, ObservableSubscriptionChangeListener,
    Role, Symbol,
};
use support::{Hub, collect};

/// A change of the symbols of a subscription, as a listener sees it.
#[derive(Debug, PartialEq)]
enum Change {
    Added(Vec<Symbol>),
    Removed(Vec<Symbol>),
    Closed,
}

/// A listener that sends every change it sees to a channel.
struct Recorder(Sender<Change>);

impl ObservableSubscriptionChangeListener for Recorder {
    fn symbols_added(&mut self, symbols: &[Symbol]) {
        let _ = self.0.send(Change::Added(symbols.to_vec()));
    }

    fn symbols_removed(&mut self, symbols: &[Symbol]) {
        let _ = self.0.send(Change::Removed(symbols.to_vec()));
    }

    fn subscription_closed(&mut self) {
        let _ = self.0.send(Change::Closed);
    }
}

fn recorder() -> (Recorder, Receiver<Change>) {
    let (sender, receiver) = mpsc::channel();
    (Recorder(sender), receiver)
}

fn next(changes: &Receiver<Change>) -> Change {
    changes
        .recv_timeout(Duration::from_secs(10))
        .expect("no change within 10 s")
}

fn none(changes: &Receiver<Change>) {
    assert_eq!(changes.recv_timeout(Duration::from_millis(300)).ok(), None);
}

#[test]
fn publisher_subscription_observes_the_symbols_of_feeds() {
    let hub = Hub::with_default_executor();
    let observed = hub.publisher.subscription::<Profile>().unwrap();
    assert!(!observed.is_closed().unwrap());
    assert_eq!(observed.event_types().unwrap(), [EventType::Profile]);
    assert!(observed.contains_event_type(EventType::Profile).unwrap());
    assert!(!observed.contains_event_type(EventType::Quote).unwrap());
    let (listener, changes) = recorder();
    let id = observed.add_change_listener(listener).unwrap();
    none(&changes);

    let subscription = hub.feed.create_subscription::<Profile>().unwrap();
    subscription.add_symbols(["AAPL"]).unwrap();
    assert_eq!(next(&changes), Change::Added(vec!["AAPL".into()]));
    subscription.add_symbols(["IBM"]).unwrap();
    assert_eq!(next(&changes), Change::Added(vec!["IBM".into()]));
    subscription.remove_symbols(["AAPL"]).unwrap();
    assert_eq!(next(&changes), Change::Removed(vec!["AAPL".into()]));

    // A listener added later is told the symbols the subscription has.
    let (late, late_changes) = recorder();
    observed.add_change_listener(late).unwrap();
    assert_eq!(next(&late_changes), Change::Added(vec!["IBM".into()]));

    // A removed listener is told so, and nothing after that.
    observed.remove_change_listener(id).unwrap();
    assert_eq!(next(&changes), Change::Closed);
    assert!(matches!(
        observed.remove_change_listener(id),
        Err(Error::InvalidArgument(_))
    ));
    subscription.add_symbols(["MSFT"]).unwrap();
    assert_eq!(next(&late_changes), Change::Added(vec!["MSFT".into()]));
    none(&changes);
}

#[test]
fn listeners_see_every_kind_of_symbol() {
    let endpoint = DXEndpoint::new_builder()
        .with_role(Role::LocalHub)
        .with_name("observable-kinds")
        .with_property("dxfeed.wildcard.enable", "true")
        .build()
        .unwrap();
    let (candle_listener, candle_changes) = recorder();
    let candles = endpoint.publisher().subscription::<Candle>().unwrap();
    candles.add_change_listener(candle_listener).unwrap();
    let (order_listener, order_changes) = recorder();
    let orders = endpoint.publisher().subscription::<Order>().unwrap();
    orders.add_change_listener(order_listener).unwrap();

    let candle = CandleSymbol::value_of("AAPL{=d}").unwrap();
    let candle_subscription = endpoint.feed().create_subscription::<Candle>().unwrap();
    candle_subscription
        .add_symbols([Symbol::time_series(&candle, 1_000)])
        .unwrap();
    assert_eq!(
        next(&candle_changes),
        Change::Added(vec![Symbol::time_series(candle, 1_000)])
    );

    let order_subscription = endpoint.feed().create_subscription::<Order>().unwrap();
    order_subscription
        .add_symbols([Symbol::indexed("AAPL", OrderSource::NTV)])
        .unwrap();
    assert_eq!(
        next(&order_changes),
        Change::Added(vec![Symbol::indexed("AAPL", OrderSource::NTV)])
    );
    order_subscription.add_symbols([Symbol::Wildcard]).unwrap();
    assert_eq!(next(&order_changes), Change::Added(vec![Symbol::Wildcard]));
}

#[test]
fn a_listener_publishes_what_feeds_ask_for() {
    let hub = Hub::with_default_executor();
    let observed = hub.publisher.subscription::<Profile>().unwrap();
    let publisher = hub.publisher.clone();
    observed
        .add_change_listener(move |symbols: &[Symbol]| {
            let profiles: Vec<Event> = symbols
                .iter()
                .filter_map(|symbol| match symbol {
                    Symbol::String(symbol) => Some(
                        Profile {
                            description: Some("Test symbol".into()),
                            ..Profile::new(symbol)
                        }
                        .into(),
                    ),
                    _ => None,
                })
                .collect();
            publisher.publish_events(&profiles).unwrap();
        })
        .unwrap();

    let subscription = hub.feed.create_subscription::<Profile>().unwrap();
    let profiles = collect(&subscription);
    subscription.add_symbols(["A:TEST"]).unwrap();
    let Ok(Event::Profile(profile)) = profiles.recv_timeout(Duration::from_secs(10)) else {
        panic!("no profile within 10 s");
    };
    assert_eq!(profile.symbol, "A:TEST");
    assert_eq!(profile.description.as_deref(), Some("Test symbol"));
}

#[test]
fn closing_tells_the_listeners() {
    let hub = Hub::with_default_executor();
    let subscription = hub.feed.create_subscription::<Profile>().unwrap();
    subscription.add_symbols(["AAPL"]).unwrap();
    let (listener, changes) = recorder();
    subscription.add_change_listener(listener).unwrap();
    assert_eq!(next(&changes), Change::Added(vec!["AAPL".into()]));
    subscription.close().unwrap();
    assert_eq!(next(&changes), Change::Closed);

    let observed = hub.publisher.subscription::<Profile>().unwrap();
    let (listener, changes) = recorder();
    observed.add_change_listener(listener).unwrap();
    hub.endpoint.close().unwrap();
    assert_eq!(next(&changes), Change::Closed);
    assert!(observed.is_closed().unwrap());
}

/// A listener that removes itself when it is first told of added symbols after its id arrives.
struct SelfRemover {
    observed: Arc<ObservableSubscription>,
    id: Receiver<ListenerId>,
    changes: Sender<Change>,
}

impl ObservableSubscriptionChangeListener for SelfRemover {
    fn symbols_added(&mut self, symbols: &[Symbol]) {
        // The first call comes from `add_change_listener` itself, before the id is known.
        if let Ok(id) = self.id.try_recv() {
            self.observed.remove_change_listener(id).unwrap();
            let _ = self.changes.send(Change::Added(symbols.to_vec()));
        }
    }

    fn subscription_closed(&mut self) {
        let _ = self.changes.send(Change::Closed);
    }
}

#[test]
fn a_listener_removes_itself() {
    let hub = Hub::with_default_executor();
    let observed = Arc::new(hub.publisher.subscription::<Profile>().unwrap());
    let (ids, id) = mpsc::channel();
    let (sender, changes) = mpsc::channel();
    let listener = SelfRemover {
        observed: observed.clone(),
        id,
        changes: sender,
    };
    ids.send(observed.add_change_listener(listener).unwrap()).unwrap();
    let subscription = hub.feed.create_subscription::<Profile>().unwrap();
    subscription.add_symbols(["AAPL"]).unwrap();
    assert_eq!(next(&changes), Change::Added(vec!["AAPL".into()]));
    // The close comes after the call that removed the listener returns.
    assert_eq!(next(&changes), Change::Closed);
    subscription.add_symbols(["IBM"]).unwrap();
    none(&changes);
}

/// Tells when it is dropped.
struct DropSignal(Sender<()>);

impl Drop for DropSignal {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}

#[test]
fn listeners_stay_until_the_endpoint_closes() {
    let hub = Hub::with_default_executor();
    let (mut recorder, changes) = recorder();
    let (signal, dropped) = mpsc::channel();
    let signal = DropSignal(signal);
    // As in Java, the subscription object is a temporary, and the listener holds the publisher.
    let publisher = hub.publisher.clone();
    hub.publisher
        .subscription::<Profile>()
        .unwrap()
        .add_change_listener(move |symbols: &[Symbol]| {
            let _held = (&signal, &publisher);
            recorder.symbols_added(symbols);
        })
        .unwrap();

    let subscription = hub.feed.create_subscription::<Profile>().unwrap();
    subscription.add_symbols(["AAPL"]).unwrap();
    assert_eq!(next(&changes), Change::Added(vec!["AAPL".into()]));
    assert_eq!(dropped.try_recv(), Err(mpsc::TryRecvError::Empty));

    // Closing the endpoint lets go of the listener, though it holds the publisher.
    hub.endpoint.close().unwrap();
    assert_eq!(dropped.recv_timeout(Duration::from_secs(10)), Ok(()));
}
