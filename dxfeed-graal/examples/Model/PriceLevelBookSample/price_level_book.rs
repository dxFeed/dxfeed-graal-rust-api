// The module keeps all the methods of the C++ header; the sample uses some of them.
#![allow(dead_code)]

use std::collections::HashMap;
use std::marker::PhantomData;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::thread;
use std::time::Duration;

use dxfeed_graal::event::{EventFlags, IndexedEventSource, Order, OrderEvent, Side};
use dxfeed_graal::model::{IndexedTxModel, IndexedTxModelBuilder};
use dxfeed_graal::{DXFeed, Symbol};

use super::price_level::PriceLevel;

/// The listener of the book: it gets the buy and the sell price levels.
pub type Listener = dyn FnMut(&[PriceLevel], &[PriceLevel]) + Send;

/// Builder class for constructing instances of `PriceLevelBook`.
pub struct PriceLevelBookBuilder {
    builder: IndexedTxModelBuilder<Order>,
    source: IndexedEventSource,
    listener: Option<Box<Listener>>,
    depth_limit: usize,
    aggregation_period_millis: i64,
}

impl PriceLevelBookBuilder {
    fn new() -> Self {
        PriceLevelBookBuilder {
            builder: IndexedTxModel::<Order>::new_builder(),
            source: IndexedEventSource::DEFAULT,
            listener: None,
            depth_limit: 0,
            aggregation_period_millis: 0,
        }
    }

    /// Sets the `DXFeed` for the model being created.
    /// The feed cannot be attached after the model has been built.
    pub fn with_feed(mut self, feed: &DXFeed) -> Self {
        self.builder = self.builder.with_feed(feed);

        self
    }

    /// Sets the subscription symbol for the model being created.
    /// The symbol cannot be added or changed after the model has been built.
    pub fn with_symbol(mut self, symbol: impl Into<Symbol>) -> Self {
        self.builder = self.builder.with_symbol(symbol);

        self
    }

    /// Sets the listener for transaction notifications.
    /// The listener cannot be changed or added once the model has been built.
    pub fn with_listener(mut self, listener: impl FnMut(&[PriceLevel], &[PriceLevel]) + Send + 'static) -> Self {
        self.listener = Some(Box::new(listener));

        self
    }

    /// Sets the source from which to subscribe for indexed events.
    /// The listener cannot be changed or added once the model has been built.
    pub fn with_source(mut self, source: IndexedEventSource) -> Self {
        self.source = source;
        self.builder = self.builder.with_sources([source]);

        self
    }

    /// Sets the depth limit.
    pub fn with_depth_limit(mut self, depth_limit: usize) -> Self {
        self.depth_limit = depth_limit;

        self
    }

    /// Sets the aggregation period in milliseconds.
    pub fn with_aggregation_period_millis(mut self, aggregation_period_millis: i64) -> Self {
        self.aggregation_period_millis = aggregation_period_millis;

        self
    }

    /// Sets the aggregation period.
    pub fn with_aggregation_period(self, aggregation_period: Duration) -> Self {
        self.with_aggregation_period_millis(aggregation_period.as_millis() as i64)
    }

    /// Builds and returns an instance of `PriceLevelBook` based on the provided parameters.
    pub fn build(self) -> dxfeed_graal::Result<PriceLevelBook> {
        PriceLevelBook::create(self)
    }
}

/// The ordering of a sorted set of price levels: `less` is the `Less` comparator of the C++ header.
pub trait Less {
    fn less(pl1: &PriceLevel, pl2: &PriceLevel) -> bool;
}

pub struct BuyLess;

impl Less for BuyLess {
    fn less(pl1: &PriceLevel, pl2: &PriceLevel) -> bool {
        pl1.get_price() > pl2.get_price()
    }
}

pub struct SellLess;

impl Less for SellLess {
    fn less(pl1: &PriceLevel, pl2: &PriceLevel) -> bool {
        pl1.get_price() < pl2.get_price()
    }
}

/// Represents a set of price levels, sorted by a comparator.
///
/// `L` is the comparator type. The levels are kept in a vector in the order of the comparator, and the levels that the
/// comparator does not tell apart (the prices are equal) are one level, as in the `std::set` of the C++ header. The
/// set has no mutex of its own: the book locks its state.
pub struct SortedPriceLevelSet<L: Less> {
    snapshot: Vec<PriceLevel>,
    price_levels: Vec<PriceLevel>,
    depth_limit: usize,
    is_changed: bool,
    _less: PhantomData<L>,
}

impl<L: Less> Default for SortedPriceLevelSet<L> {
    fn default() -> Self {
        SortedPriceLevelSet {
            snapshot: Vec::new(),
            price_levels: Vec::new(),
            depth_limit: 0,
            is_changed: false,
            _less: PhantomData,
        }
    }
}

impl<L: Less> SortedPriceLevelSet<L> {
    fn is_depth_limit_unbounded(&self) -> bool {
        let temp = self.depth_limit;

        temp == 0 || temp == usize::MAX
    }

    fn is_price_level_count_within_depth_limit(&self) -> bool {
        self.price_levels.len() <= self.depth_limit
    }

    fn is_price_level_within_depth_limit(&self, pl: &PriceLevel) -> bool {
        match self.snapshot.last() {
            None => true,
            Some(last) => !L::less(last, pl),
        }
    }

    fn update_snapshot(&mut self) {
        self.is_changed = false;
        self.snapshot.clear();

        let limit = if self.is_depth_limit_unbounded() {
            usize::MAX
        } else {
            self.depth_limit
        };

        self.snapshot.extend(self.price_levels.iter().take(limit).cloned());
    }

    fn mark_as_changed_if_needed(&mut self, pl: &PriceLevel) {
        if self.is_changed {
            return;
        }

        if self.is_depth_limit_unbounded()
            || self.is_price_level_count_within_depth_limit()
            || self.is_price_level_within_depth_limit(pl)
        {
            self.is_changed = true;
        }
    }

    /// The position of the level that the comparator does not tell apart from `pl` (`Ok`), or where `pl` is to be
    /// inserted (`Err`): `std::set::find`.
    fn find(&self, pl: &PriceLevel) -> Result<usize, usize> {
        let position = self.price_levels.partition_point(|existing| L::less(existing, pl));

        match self.price_levels.get(position) {
            Some(existing) if !L::less(pl, existing) => Ok(position),
            _ => Err(position),
        }
    }

    /// Returns a value indicating whether this set has changed.
    pub fn is_changed(&self) -> bool {
        self.is_changed
    }

    /// Sets the depth limit.
    pub fn set_depth_limit(&mut self, depth_limit: usize) {
        if self.depth_limit == depth_limit {
            return;
        }

        self.depth_limit = depth_limit;
        self.is_changed = true;
    }

    /// Returns the depth limit.
    pub fn get_depth_limit(&self) -> usize {
        self.depth_limit
    }

    /// Inserts a new price level or increases the size of an existing price level.
    pub fn insert_or_increase(&mut self, pl: PriceLevel) {
        match self.find(&pl) {
            Ok(position) => {
                let mut existing = self.price_levels.remove(position);

                existing.set_size(existing.get_size() + pl.get_size());
                self.price_levels.insert(position, existing.clone());
                self.mark_as_changed_if_needed(&existing);
            }
            Err(position) => {
                self.price_levels.insert(position, pl.clone());
                self.mark_as_changed_if_needed(&pl);
            }
        }
    }

    /// Removes existing price level or decreases the size of an existing price level.
    pub fn erase_or_decrease(&mut self, pl: &PriceLevel) {
        if let Ok(position) = self.find(pl) {
            let mut existing = self.price_levels.remove(position);
            let new_size = existing.get_size() - pl.get_size();

            if new_size > 0.0 {
                existing.set_size(new_size);
                self.price_levels.insert(position, existing.clone());
            }

            self.mark_as_changed_if_needed(&existing);
        }
    }

    /// Clears price levels from the set by source.
    pub fn clear_by_source(&mut self, source: IndexedEventSource) {
        let size = self.price_levels.len();

        self.price_levels
            .retain(|price_level| price_level.get_event_source() != source);
        self.is_changed = self.price_levels.len() != size;
    }

    /// Converts the set to a vector.
    ///
    /// Returns the vector of price levels.
    #[allow(clippy::wrong_self_convention)] // It refreshes the snapshot, as the C++ method does.
    pub fn to_vector(&mut self) -> Vec<PriceLevel> {
        if self.is_changed {
            self.update_snapshot();
        }

        self.snapshot.clone()
    }
}

/// `Timer::runOnce` of the C++ API: runs a task once after a delay on a thread of its own, unless it is stopped
/// (or the timer is dropped) first.
struct Timer {
    stop: mpsc::Sender<()>,
    running: Arc<AtomicBool>,
}

impl Timer {
    fn run_once(task: impl FnOnce() + Send + 'static, delay: Duration) -> Timer {
        let (stop, stopped) = mpsc::channel();
        let running = Arc::new(AtomicBool::new(true));
        let thread_running = Arc::clone(&running);

        thread::spawn(move || {
            if let Err(RecvTimeoutError::Timeout) = stopped.recv_timeout(delay) {
                // A panic of the task must not leave the timer running.
                let _ = panic::catch_unwind(AssertUnwindSafe(task));
            }

            thread_running.store(false, Ordering::SeqCst);
        });

        Timer { stop, running }
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Stops the timer without waiting for its thread.
    fn stop(&self) {
        let _ = self.stop.send(());
        self.running.store(false, Ordering::SeqCst);
    }
}

/// What the book guards with its mutex (`mtx_`) and shares with the listener of the model and with the timer.
struct State {
    orders_by_index: HashMap<i64, Order>,
    buy_price_levels: SortedPriceLevelSet<BuyLess>,
    sell_price_levels: SortedPriceLevelSet<SellLess>,
    listener: Box<Listener>,
    depth_limit: usize,
    aggregation_period_millis: i64,
    task_scheduled: bool,
    task_timer: Option<Timer>,
    /// The book itself, for the timer: it does not keep the book alive.
    this: Weak<Mutex<State>>,
}

/// Locks the state; a panic of a listener does not make the book unusable.
fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

impl State {
    fn shall_add(order: &Order) -> bool {
        order.has_size() && !order.event_flags.contains(EventFlags::REMOVE_EVENT)
    }

    fn events_received(&mut self, source: IndexedEventSource, events: &[Order], is_snapshot: bool) {
        if !self.update(source, events, is_snapshot) {
            return;
        }

        if is_snapshot || self.aggregation_period_millis == 0 {
            self.try_cancel_task();
            self.notify_listeners();
        } else {
            self.schedule_task_if_needed(Duration::from_millis(self.aggregation_period_millis.unsigned_abs()));
        }
    }

    fn notify_listeners(&mut self) {
        let buy = self.buy_price_levels.to_vector();
        let sell = self.sell_price_levels.to_vector();

        (self.listener)(&buy, &sell);
        self.task_scheduled = false;
    }

    fn schedule_task_if_needed(&mut self, delay: Duration) {
        if !self.task_scheduled {
            self.task_scheduled = true;

            let this = self.this.clone();

            self.task_timer = Some(Timer::run_once(
                move || {
                    if let Some(book) = this.upgrade() {
                        lock(&book).notify_listeners();
                    }
                },
                delay,
            ));
        }
    }

    fn reschedule_task_if_needed(&mut self, delay: Duration) {
        if self.try_cancel_task() && !delay.is_zero() {
            self.schedule_task_if_needed(delay);
        }
    }

    fn try_cancel_task(&mut self) -> bool {
        if self.task_scheduled && self.task_timer.as_ref().is_some_and(Timer::is_running) {
            if let Some(timer) = self.task_timer.take() {
                timer.stop();
            }

            self.task_scheduled = false;

            return true;
        }

        false
    }

    fn update(&mut self, source: IndexedEventSource, events: &[Order], is_snapshot: bool) -> bool {
        if is_snapshot {
            self.clear_by_source(source);
        }

        for order in events {
            if let Some(removed) = self.orders_by_index.remove(&order.index) {
                if removed.order_side() == Side::Buy {
                    self.buy_price_levels.erase_or_decrease(&PriceLevel::new(&removed));
                } else {
                    self.sell_price_levels.erase_or_decrease(&PriceLevel::new(&removed));
                }
            }

            if Self::shall_add(order) {
                self.orders_by_index.insert(order.index, order.clone());

                if order.order_side() == Side::Buy {
                    self.buy_price_levels.insert_or_increase(PriceLevel::new(order));
                } else {
                    self.sell_price_levels.insert_or_increase(PriceLevel::new(order));
                }
            }
        }

        self.buy_price_levels.is_changed() || self.sell_price_levels.is_changed()
    }

    fn clear_by_source(&mut self, source: IndexedEventSource) {
        self.orders_by_index
            .retain(|_, order| IndexedEventSource::from_id(order.source()).unwrap_or_default() != source);

        self.buy_price_levels.clear_by_source(source);
        self.sell_price_levels.clear_by_source(source);
    }
}

/// Represents a price level book (market by price) that aggregates individual orders (market by order).
/// For this model, use market by order sources such as `OrderSource::NTV`, `OrderSource::GLBX`, etc.
///
/// The C++ header is a template over the order type; this one takes `Order`, as the price levels keep the event symbol
/// of the order.
pub struct PriceLevelBook {
    state: Arc<Mutex<State>>,
    indexed_tx_model: IndexedTxModel<Order>,
}

impl PriceLevelBook {
    fn create(builder: PriceLevelBookBuilder) -> dxfeed_graal::Result<PriceLevelBook> {
        let state = Arc::new_cyclic(|this| {
            let mut buy_price_levels = SortedPriceLevelSet::default();
            let mut sell_price_levels = SortedPriceLevelSet::default();

            buy_price_levels.set_depth_limit(builder.depth_limit);
            sell_price_levels.set_depth_limit(builder.depth_limit);

            Mutex::new(State {
                orders_by_index: HashMap::new(),
                buy_price_levels,
                sell_price_levels,
                listener: builder.listener.unwrap_or_else(|| Box::new(|_, _| {})),
                depth_limit: builder.depth_limit,
                aggregation_period_millis: builder.aggregation_period_millis,
                task_scheduled: false,
                task_timer: None,
                this: this.clone(),
            })
        });
        let m = Arc::downgrade(&state);
        let indexed_tx_model = builder
            .builder
            .with_listener(move |source, events, is_snapshot| {
                if let Some(state) = m.upgrade() {
                    let events: Vec<Order> = events.map(|order| order.to_owned()).collect();

                    lock(&state).events_received(source, &events, is_snapshot);
                }
            })
            .build()?;

        Ok(PriceLevelBook {
            state,
            indexed_tx_model,
        })
    }

    /// Creates a new builder instance for constructing a `PriceLevelBook`.
    pub fn new_builder() -> PriceLevelBookBuilder {
        PriceLevelBookBuilder::new()
    }

    /// Returns the depth limit of the book.
    pub fn get_depth_limit(&self) -> usize {
        lock(&self.state).depth_limit
    }

    /// Sets the depth limit of the book.
    pub fn set_depth_limit(&self, depth_limit: usize) {
        let mut state = lock(&self.state);

        if depth_limit == state.depth_limit {
            return;
        }

        state.depth_limit = depth_limit;
        state.buy_price_levels.set_depth_limit(depth_limit);
        state.sell_price_levels.set_depth_limit(depth_limit);
        state.try_cancel_task();
        state.notify_listeners();
    }

    /// Returns the aggregation period of the book, in milliseconds.
    pub fn get_aggregation_period(&self) -> i64 {
        lock(&self.state).aggregation_period_millis
    }

    /// Sets the aggregation period in milliseconds.
    pub fn set_aggregation_period_millis(&self, aggregation_period_millis: i64) {
        let mut state = lock(&self.state);

        if aggregation_period_millis == state.aggregation_period_millis {
            return;
        }

        state.aggregation_period_millis = aggregation_period_millis;
        state.reschedule_task_if_needed(Duration::from_millis(aggregation_period_millis.unsigned_abs()));
    }

    /// Sets the aggregation period.
    pub fn set_aggregation_period(&self, aggregation_period: Duration) {
        self.set_aggregation_period_millis(aggregation_period.as_millis() as i64);
    }

    /// Closes this book and makes it _permanently detached_.
    pub fn close(&self) {
        let _ = self.indexed_tx_model.close();
    }
}

impl Drop for PriceLevelBook {
    fn drop(&mut self) {
        self.close();
    }
}
