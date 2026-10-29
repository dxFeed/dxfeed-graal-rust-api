//! The `LOCAL_HUB` fixture: events published to a hub reach its listeners on the test thread.

#![allow(dead_code)] // Each test binary uses a part.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};

use dxfeed_graal::event::{Quote, Trade};
use dxfeed_graal::{Endpoint, Event, Feed, ManualExecutor, Publisher, Role, Subscription};

/// A `LOCAL_HUB` endpoint with a name unique in this process, its manual executor, feed and publisher.
pub struct Hub {
    pub endpoint: Endpoint,
    pub executor: ManualExecutor,
    pub feed: Feed,
    pub publisher: Publisher,
}

impl Hub {
    pub fn new() -> Self {
        Self::with_executor(Some(ManualExecutor::new().unwrap()))
    }

    /// A hub whose listeners run on the endpoint's default executor threads.
    pub fn with_default_executor() -> Self {
        Self::with_executor(None)
    }

    fn with_executor(executor: Option<ManualExecutor>) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let mut builder = Endpoint::builder()
            .with_role(Role::LocalHub)
            .with_name(&format!("test-hub-{}", NEXT.fetch_add(1, Ordering::Relaxed)));
        if let Some(executor) = &executor {
            builder = builder.with_executor(executor);
        }
        let endpoint = builder.build().unwrap();
        let feed = endpoint.feed().unwrap();
        let publisher = endpoint.publisher().unwrap();
        // An unused executor for default-executor hubs keeps the fields uniform.
        let executor = executor.unwrap_or_else(|| ManualExecutor::new().unwrap());
        Hub {
            endpoint,
            executor,
            feed,
            publisher,
        }
    }

    /// Publishes `events` and runs the resulting listener calls on this thread.
    pub fn publish_and_process(&self, events: &[Event]) {
        self.publisher.publish(events).unwrap();
        self.executor.process_all_pending().unwrap();
    }
}

/// Adds a listener that sends copies of the received events to the returned channel; assertions belong in the test.
pub fn collect(subscription: &Subscription) -> Receiver<Event> {
    let (sender, receiver) = mpsc::channel();
    subscription
        .add_listener(move |events| {
            for event in events {
                if let Some(event) = event.to_owned() {
                    let _ = sender.send(event);
                }
            }
        })
        .unwrap();
    receiver
}

/// A quote with a distinct non-zero value in every field.
pub fn quote(symbol: &str) -> Quote {
    Quote {
        event_time: 1_700_000_000_500,
        time_millis_sequence: (123 << 22) | 7,
        time_nano_part: 456,
        bid_time: 1_700_000_000_000,
        bid_exchange_code: 'Q',
        bid_price: 189.5,
        bid_size: 100.0,
        ask_time: 1_700_000_001_000, // Whole seconds: the SDK truncates the time to seconds.
        ask_exchange_code: 'N',
        ask_price: 189.75,
        ask_size: 200.0,
        ..Quote::new(symbol)
    }
}

/// A trade with a distinct non-zero value in every field.
pub fn trade(symbol: &str) -> Trade {
    Trade {
        event_time: 1_700_000_000_500,
        time_sequence: (1_700_000_000_000 << 22) | 9,
        time_nano_part: 321,
        exchange_code: 'Q',
        price: 190.25,
        change: 0.5,
        size: 300.0,
        day_id: 19_675,
        day_volume: 1_000_000.0,
        day_turnover: 190_000_000.0,
        flags: 4,
        trade_id: 42,
        ..Trade::new(symbol)
    }
}

/// [`quote`] with another bid price: the hub does not deliver a quote identical to the last one.
pub fn quote_with_bid(symbol: &str, bid_price: f64) -> Quote {
    Quote {
        bid_price,
        ..quote(symbol)
    }
}
