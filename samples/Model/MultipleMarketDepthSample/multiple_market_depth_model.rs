// The module keeps all the methods of the C++ header; the sample uses some of them.
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use dxfeed_graal::event::{EventKind, IndexedEventSource, OrderEvent};
use dxfeed_graal::model::MarketDepthModel;
use dxfeed_graal::{DXFeed, Error, Symbol};

use super::order_book::OrderBook;

/// The listener of the books: it gets the symbol and the book whenever the book changes.
pub type Listener<O> = dyn Fn(&Symbol, &OrderBook<O>) + Send + Sync;

/// The key of a symbol in the maps: the event symbol and the source of an `IndexedEventSubscriptionSymbol`.
type Key = (String, IndexedEventSource);

/// The event symbol and the source of an `IndexedEventSubscriptionSymbol` (`getEventSymbol`, `getSource`).
fn split(symbol: &Symbol) -> dxfeed_graal::Result<(&Symbol, IndexedEventSource)> {
    match symbol {
        Symbol::IndexedEventSubscription { symbol, source } => Ok((symbol, *source)),
        _ => Err(Error::InvalidArgument(format!(
            "{symbol} is not an indexed event subscription symbol"
        ))),
    }
}

fn key(symbol: &Symbol) -> dxfeed_graal::Result<Key> {
    let (event_symbol, source) = split(symbol)?;

    Ok((event_symbol.to_string(), source))
}

/// What the builder passes to the models: the settings of the `MarketDepthModel` builder, which Rust consumes by
/// `build`, so the model makes a builder of them for each symbol.
#[derive(Clone, Default)]
struct Settings {
    feed: Option<DXFeed>,
    depth_limit: Option<usize>,
    aggregation_period: Option<Duration>,
}

/// A builder class for creating instances of `MultipleMarketDepthModel`.
pub struct MultipleMarketDepthModelBuilder<O: OrderEvent + EventKind + Sync> {
    settings: Settings,
    listener: Option<Box<Listener<O>>>,
}

impl<O: OrderEvent + EventKind + Sync> MultipleMarketDepthModelBuilder<O> {
    fn new() -> Self {
        MultipleMarketDepthModelBuilder {
            settings: Settings::default(),
            listener: None,
        }
    }

    /// Sets the feed for the market depth model.
    pub fn with_feed(mut self, feed: &DXFeed) -> Self {
        self.settings.feed = Some(feed.clone());

        self
    }

    /// Sets the depth limit for the market depth model.
    pub fn with_depth_limit(mut self, depth_limit: usize) -> Self {
        self.settings.depth_limit = Some(depth_limit);

        self
    }

    /// Sets the aggregation period for the market depth model, in milliseconds.
    pub fn with_aggregation_period_millis(self, aggregation_period_millis: u64) -> Self {
        self.with_aggregation_period(Duration::from_millis(aggregation_period_millis))
    }

    /// Sets the aggregation period for the market depth model.
    pub fn with_aggregation_period(mut self, aggregation_period: Duration) -> Self {
        self.settings.aggregation_period = Some(aggregation_period);

        self
    }

    /// Sets the listener for updates to the order book.
    pub fn with_listener(
        mut self,
        on_events_received: impl Fn(&Symbol, &OrderBook<O>) + Send + Sync + 'static,
    ) -> Self {
        self.listener = Some(Box::new(on_events_received));

        self
    }

    /// Builds and returns an instance of `MultipleMarketDepthModel`.
    pub fn build(self) -> MultipleMarketDepthModel<O> {
        MultipleMarketDepthModel::new(self)
    }
}

/// The models and the latest books, shared with the listeners of the models.
struct Shared<O: OrderEvent + EventKind + Sync> {
    state: Mutex<State<O>>,
    listener: Box<Listener<O>>,
}

struct State<O: OrderEvent + EventKind + Sync> {
    models: HashMap<Key, MarketDepthModel<O>>,
    books: HashMap<Key, Arc<OrderBook<O>>>,
}

/// This class manages multiple market depth models for different symbols.
/// It provides methods to add and remove symbols and to retrieve the current order book for a symbol.
///
/// `O` is the type of order.
pub struct MultipleMarketDepthModel<O: OrderEvent + EventKind + Sync> {
    settings: Settings,
    shared: Arc<Shared<O>>,
}

impl<O: OrderEvent + EventKind + Sync> MultipleMarketDepthModel<O> {
    fn new(builder: MultipleMarketDepthModelBuilder<O>) -> Self {
        MultipleMarketDepthModel {
            settings: builder.settings,
            shared: Arc::new(Shared {
                state: Mutex::new(State {
                    models: HashMap::new(),
                    books: HashMap::new(),
                }),
                listener: builder.listener.unwrap_or_else(|| Box::new(|_, _| {})),
            }),
        }
    }

    /// Returns a new builder.
    pub fn new_builder() -> MultipleMarketDepthModelBuilder<O> {
        MultipleMarketDepthModelBuilder::new()
    }

    fn create_model(&self, symbol: &Symbol) -> dxfeed_graal::Result<MarketDepthModel<O>> {
        let (event_symbol, source) = split(symbol)?;
        let mut builder = MarketDepthModel::<O>::new_builder();

        if let Some(feed) = &self.settings.feed {
            builder = builder.with_feed(feed);
        }

        if let Some(depth_limit) = self.settings.depth_limit {
            builder = builder.with_depth_limit(depth_limit);
        }

        if let Some(aggregation_period) = self.settings.aggregation_period {
            builder = builder.with_aggregation_period(aggregation_period);
        }

        let this = Arc::downgrade(&self.shared);
        let symbol = symbol.clone();

        builder
            .with_symbol(event_symbol)
            .with_sources([source])
            .with_listener(move |buy, sell| on_book(&this, &symbol, buy, sell))
            .build()
    }

    /// Adds a symbol to the model.
    pub fn add_symbol(&self, symbol: Symbol) -> dxfeed_graal::Result<()> {
        let key = key(&symbol)?;
        let mut state = self.shared.state.lock().unwrap();

        if state.models.contains_key(&key) {
            return Ok(());
        }

        let model = self.create_model(&symbol)?;

        state.models.insert(key, model);

        Ok(())
    }

    /// Removes a symbol from the model.
    ///
    /// Add and remove the symbols on one thread: a book of a symbol removed and added again on another thread at the
    /// same time may come from the removed model.
    pub fn remove_symbol(&self, symbol: &Symbol) {
        let Ok(key) = key(symbol) else {
            return;
        };
        let model = {
            let mut state = self.shared.state.lock().unwrap();
            let model = state.models.remove(&key);

            if model.is_some() {
                state.books.remove(&key);
            }

            model
        };

        // Closed without the lock, as the model waits for its listener, which takes the lock.
        drop(model);
    }

    /// Tries to get the order book for a given symbol.
    ///
    /// Returns the book if there is one; otherwise `None`.
    pub fn try_get_book(&self, symbol: &Symbol) -> Option<Arc<OrderBook<O>>> {
        let key = key(symbol).ok()?;

        self.shared.state.lock().unwrap().books.get(&key).cloned()
    }
}

impl<O: OrderEvent + EventKind + Sync> Drop for MultipleMarketDepthModel<O> {
    fn drop(&mut self) {
        // The models are closed without the lock, as they wait for their listeners, which take the lock.
        let models = std::mem::take(&mut self.shared.state.lock().unwrap().models);

        drop(models);
    }
}

/// Keeps the new book of the symbol and passes it to the listener, unless the symbol was removed.
fn on_book<O: OrderEvent + EventKind + Sync>(shared: &Weak<Shared<O>>, symbol: &Symbol, buy: &[O], sell: &[O]) {
    let Some(shared) = shared.upgrade() else {
        return;
    };
    let Ok(key) = key(symbol) else {
        return;
    };
    let book = {
        let mut state = shared.state.lock().unwrap();

        if !state.models.contains_key(&key) {
            return;
        }

        let book = Arc::new(OrderBook::new(buy, sell));

        state.books.insert(key, Arc::clone(&book));

        book
    };

    // Called without the lock, so that the listener may get other books.
    (shared.listener)(symbol, &book);
}
