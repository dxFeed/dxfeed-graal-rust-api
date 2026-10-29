//! The `LOCAL_HUB` fixture (KTD11): events published to a hub reach its listeners on the test thread.

#![allow(dead_code)] // Each test binary uses a part.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};

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
