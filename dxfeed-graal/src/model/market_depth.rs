//! The market depth model (`MarketDepthModel`): the sorted buy and sell sides of an order book, built on an
//! [`IndexedTxModel`] in Rust, as the C++ API builds it.

use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::panic::{self, AssertUnwindSafe};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle, ThreadId};
use std::time::{Duration, Instant};

use super::{IndexedTxModel, IndexedTxModelBuilder};
use crate::error::Result;
use crate::event::{EventFlags, EventKind, Events, IndexedEventSource, OrderEvent, Scope, Side};
use crate::feed::DXFeed;
use crate::isolate;
use crate::symbol::Symbol;

/// The listener of a market depth model: the buy side, then the sell side.
type DepthClosure<O> = dyn FnMut(&[O], &[O]) + Send;

/// A model of the order book of a symbol: the buy orders sorted from the best (highest) price down and the sell orders
/// from the best (lowest) price up, at most [`get_depth_limit`](Self::get_depth_limit) of each (`MarketDepthModel`). Build it
/// with [`MarketDepthModel::new_builder`]; it needs a symbol and a feed.
///
/// The listener gets both sides whenever an update changes the orders within the depth limit: at once for a
/// snapshot, and for the other updates once per [`get_aggregation_period`](Self::get_aggregation_period), if it is not zero.
/// At one price, the price levels (the aggregate orders) go before the individual orders; the larger price levels go
/// first, the individual orders in the order of their time.
///
/// Dropping the model closes it; dropping it from another thread waits for a running call of the listener, so do not
/// hold a lock its closure takes while dropping it. It keeps the endpoint of its feed open.
///
/// ```no_run
/// use std::time::Duration;
///
/// use dxfeed_graal::DXFeed;
/// use dxfeed_graal::event::{Order, OrderSource};
/// use dxfeed_graal::model::MarketDepthModel;
///
/// # fn main() -> dxfeed_graal::Result<()> {
/// let _model = MarketDepthModel::<Order>::new_builder()
///     .with_feed(DXFeed::get_instance()?)
///     .with_symbol("AAPL")
///     .with_sources([OrderSource::NTV])
///     .with_depth_limit(10)
///     .with_aggregation_period(Duration::from_millis(500))
///     .with_listener(|buy, sell| {
///         for (bid, ask) in buy.iter().zip(sell) {
///             println!("{} x {}  |  {} x {}", bid.price, bid.size, ask.price, ask.size);
///         }
///     })
///     .build()?;
/// # Ok(())
/// # }
/// ```
pub struct MarketDepthModel<O: OrderEvent + EventKind + Sync> {
    shared: Arc<Shared<O>>,
    /// Taken by `Drop` only.
    tx: Option<IndexedTxModel<O>>,
}

impl<O: OrderEvent + EventKind + Sync> MarketDepthModel<O> {
    /// A builder of a model (`newBuilder`).
    #[doc(alias = "newBuilder")]
    pub fn new_builder() -> MarketDepthModelBuilder<O> {
        MarketDepthModelBuilder {
            builder: IndexedTxModel::new_builder(),
            depth_limit: 0,
            aggregation_period: Duration::ZERO,
            listener: None,
        }
    }

    /// The most orders of each side the listener gets; `0` means all of them (`getDepthLimit`).
    #[doc(alias = "getDepthLimit")]
    pub fn get_depth_limit(&self) -> usize {
        crate::lock(&self.shared.state).depth_limit
    }

    /// Changes the depth limit and, if it is new, calls the listener with the sides it gives (`setDepthLimit`).
    #[doc(alias = "setDepthLimit")]
    pub fn set_depth_limit(&self, depth_limit: usize) {
        {
            let mut state = crate::lock(&self.shared.state);
            if state.depth_limit == depth_limit {
                return;
            }
            state.depth_limit = depth_limit;
            state.buy.changed = true;
            state.sell.changed = true;
            state.deadline = None;
        }
        self.shared.notify();
    }

    /// How long the model gathers the updates other than snapshots before it calls the listener; zero means at once
    /// (`getAggregationPeriod`).
    #[doc(alias = "getAggregationPeriod")]
    pub fn get_aggregation_period(&self) -> Duration {
        crate::lock(&self.shared.state).aggregation_period
    }

    /// Changes the aggregation period (`setAggregationPeriod`). A call of the listener that waits is moved to the new
    /// period from now, or made at once for zero.
    #[doc(alias = "setAggregationPeriod")]
    pub fn set_aggregation_period(&self, period: Duration) {
        let mut state = crate::lock(&self.shared.state);
        if state.aggregation_period == period {
            return;
        }
        state.aggregation_period = period;
        if state.deadline.is_some() {
            state.deadline = Some(Instant::now() + period);
            self.shared.wake.notify_all();
        }
    }

    /// Closes the model for good: it detaches from the feed and calls the listener no more (`close`). It waits for a
    /// call of the listener in progress on another thread; called from the listener, it does not wait. Closing it
    /// again does nothing.
    pub fn close(&self) -> Result<()> {
        {
            let mut state = crate::lock(&self.shared.state);
            state.closed = true;
            state.deadline = None;
        }
        self.shared.wake.notify_all();
        let result = self.tx.as_ref().map_or(Ok(()), IndexedTxModel::close);
        if *crate::lock(&self.shared.notifying) != Some(thread::current().id()) {
            // A call that started before `closed` was set ends before the lock is taken.
            drop(crate::lock(&self.shared.listener));
        }
        result
    }
}

impl<O: OrderEvent + EventKind + Sync> Drop for MarketDepthModel<O> {
    fn drop(&mut self) {
        if isolate::exiting() {
            return;
        }
        crate::error_manager::report_release("MarketDepthModel", self.close());
        let timer = crate::lock(&self.shared.state).timer.take();
        let tx = self.tx.take();
        if *crate::lock(&self.shared.notifying) == Some(thread::current().id()) {
            // Dropped by the listener: the timer thread or a call of the transactional model's listener may wait for
            // the call in progress, so another thread waits for them.
            let _ = thread::Builder::new()
                .name("dxfeed-graal-market-depth-drop".into())
                .spawn(move || release(tx, timer));
        } else {
            release(tx, timer);
        }
    }
}

/// Releases the transactional model, which waits for a running call of its listener, and waits for the timer thread.
fn release<O: OrderEvent + EventKind + Sync>(tx: Option<IndexedTxModel<O>>, timer: Option<JoinHandle<()>>) {
    drop(tx);
    if let Some(timer) = timer {
        let _ = timer.join();
    }
}

impl<O: OrderEvent + EventKind + Sync> fmt::Debug for MarketDepthModel<O> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = crate::lock(&self.shared.state);
        f.debug_struct("MarketDepthModel")
            .field("event_type", &O::TYPE)
            .field("depth_limit", &state.depth_limit)
            .field("aggregation_period", &state.aggregation_period)
            .field("closed", &state.closed)
            .finish_non_exhaustive()
    }
}

/// The builder of a [`MarketDepthModel`] (`MarketDepthModel.Builder`).
pub struct MarketDepthModelBuilder<O: OrderEvent + EventKind + Sync> {
    builder: IndexedTxModelBuilder<O>,
    depth_limit: usize,
    aggregation_period: Duration,
    listener: Option<Box<DepthClosure<O>>>,
}

impl<O: OrderEvent + EventKind + Sync> MarketDepthModelBuilder<O> {
    /// The feed the model attaches to when it is built (`withFeed`).
    #[doc(alias = "withFeed")]
    pub fn with_feed(mut self, feed: &DXFeed) -> Self {
        self.builder = self.builder.with_feed(feed);
        self
    }

    /// The symbol of the order book (`withSymbol`).
    #[doc(alias = "withSymbol")]
    pub fn with_symbol(mut self, symbol: impl Into<Symbol>) -> Self {
        self.builder = self.builder.with_symbol(symbol);
        self
    }

    /// The sources of the orders; none, the default, means all the sources (`withSources`).
    #[doc(alias = "withSources")]
    pub fn with_sources(mut self, sources: impl IntoIterator<Item = IndexedEventSource>) -> Self {
        self.builder = self.builder.with_sources(sources);
        self
    }

    /// The most orders of each side the listener gets; `0`, the default, means all of them (`withDepthLimit`).
    #[doc(alias = "withDepthLimit")]
    pub fn with_depth_limit(mut self, depth_limit: usize) -> Self {
        self.depth_limit = depth_limit;
        self
    }

    /// How long the model gathers the updates other than snapshots before it calls the listener; zero, the default,
    /// means at once (`withAggregationPeriod`).
    #[doc(alias = "withAggregationPeriod")]
    pub fn with_aggregation_period(mut self, period: Duration) -> Self {
        self.aggregation_period = period;
        self
    }

    /// The listener of the order book (`withListener`): it gets the buy orders, then the sell orders, sorted from the
    /// best price, at most the depth limit of each. A panic in `f` is caught and goes to
    /// [`ErrorManager`](crate::ErrorManager).
    #[doc(alias = "withListener")]
    pub fn with_listener(mut self, f: impl FnMut(&[O], &[O]) + Send + 'static) -> Self {
        self.listener = Some(Box::new(f));
        self
    }

    /// Builds the model; it attaches to the feed of [`with_feed`](Self::with_feed), if any (`build`).
    ///
    /// Returns an error if the symbol is not set.
    pub fn build(self) -> Result<MarketDepthModel<O>> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                by_index: HashMap::new(),
                buy: BookSide::default(),
                sell: BookSide::default(),
                depth_limit: self.depth_limit,
                aggregation_period: self.aggregation_period,
                deadline: None,
                closed: false,
                timer: None,
            }),
            wake: Condvar::new(),
            listener: Mutex::new(self.listener),
            notifying: Mutex::new(None),
            again: Mutex::new(false),
        });
        let receiver = Arc::clone(&shared);
        let mut builder = self.builder;
        builder.settings.listener = Some(Box::new(move |source, events: Events<'_>, is_snapshot| {
            let orders = events
                .filter_map(|event| event.to_owned())
                .filter_map(|event| O::try_from(event).ok())
                .collect();
            receiver.received(source, orders, is_snapshot);
        }));
        let tx = builder.build()?;
        Ok(MarketDepthModel { shared, tx: Some(tx) })
    }
}

impl<O: OrderEvent + EventKind + Sync> fmt::Debug for MarketDepthModelBuilder<O> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MarketDepthModelBuilder")
            .field("event_type", &O::TYPE)
            .field("depth_limit", &self.depth_limit)
            .field("aggregation_period", &self.aggregation_period)
            .finish_non_exhaustive()
    }
}

/// The state that the model, its transactional model listener and its timer thread share.
struct Shared<O: OrderEvent + EventKind + Sync> {
    state: Mutex<State<O>>,
    /// Wakes the timer thread when the deadline changes or the model closes.
    wake: Condvar,
    /// Taken for each call of the listener, so that the calls do not overlap and `close` can wait for them; taken
    /// before `state` when both are.
    listener: Mutex<Option<Box<DepthClosure<O>>>>,
    /// The thread that calls the listener now, so that the listener can call the model.
    notifying: Mutex<Option<ThreadId>>,
    /// Whether the listener changed the model, so that it is called again after the call in progress.
    again: Mutex<bool>,
}

impl<O: OrderEvent + EventKind + Sync> Shared<O> {
    /// Applies a transaction or a snapshot from the transactional model, and calls the listener now or later.
    fn received(self: &Arc<Self>, source: IndexedEventSource, orders: Vec<O>, is_snapshot: bool) {
        {
            let mut state = crate::lock(&self.state);
            if state.closed || !state.update(source, orders, is_snapshot) {
                return;
            }
            if !is_snapshot && !state.aggregation_period.is_zero() && self.schedule(&mut state) {
                return;
            }
            state.deadline = None;
        }
        self.notify();
    }

    /// Schedules a call of the listener after the aggregation period, unless one waits; `false` if the timer thread
    /// cannot start.
    fn schedule(self: &Arc<Self>, state: &mut State<O>) -> bool {
        if state.deadline.is_some() {
            return true;
        }
        if state.timer.is_none() {
            let shared = Arc::clone(self);
            match thread::Builder::new()
                .name("dxfeed-graal-market-depth".into())
                .spawn(move || shared.run_timer())
            {
                Ok(timer) => state.timer = Some(timer),
                Err(_) => return false,
            }
        }
        state.deadline = Some(Instant::now() + state.aggregation_period);
        self.wake.notify_all();
        true
    }

    /// The timer thread: calls the listener at each deadline until the model closes.
    fn run_timer(&self) {
        let mut state = crate::lock(&self.state);
        while !state.closed {
            match state.deadline {
                None => state = self.wake.wait(state).unwrap_or_else(|e| e.into_inner()),
                Some(deadline) => {
                    let now = Instant::now();
                    if now < deadline {
                        state = self
                            .wake
                            .wait_timeout(state, deadline - now)
                            .unwrap_or_else(|e| e.into_inner())
                            .0;
                    } else {
                        drop(state);
                        self.notify();
                        state = crate::lock(&self.state);
                    }
                }
            }
        }
    }

    /// Calls the listener with the sides, unless the model is closed. Called from the listener, it calls it again
    /// after the call in progress instead.
    fn notify(&self) {
        let current = thread::current().id();
        if *crate::lock(&self.notifying) == Some(current) {
            *crate::lock(&self.again) = true;
            return;
        }
        let mut listener = crate::lock(&self.listener);
        loop {
            let (buy, sell) = {
                let mut state = crate::lock(&self.state);
                if state.closed {
                    return;
                }
                state.deadline = None;
                let limit = state.depth_limit;
                (state.buy.snapshot(limit), state.sell.snapshot(limit))
            };
            let Some(listener) = listener.as_mut() else {
                return;
            };
            *crate::lock(&self.notifying) = Some(current);
            crate::error_manager::report_panic(
                "MarketDepthModel",
                "a market depth model listener",
                panic::catch_unwind(AssertUnwindSafe(|| listener(&buy, &sell))),
            );
            *crate::lock(&self.notifying) = None;
            if !std::mem::take(&mut *crate::lock(&self.again)) {
                return;
            }
        }
    }
}

/// The orders of the book.
struct State<O: OrderEvent + EventKind + Sync> {
    /// Every order of both sides by its index.
    by_index: HashMap<i64, Arc<O>>,
    buy: BookSide<O, true>,
    sell: BookSide<O, false>,
    depth_limit: usize,
    aggregation_period: Duration,
    /// When the timer thread calls the listener; `None` if no call waits.
    deadline: Option<Instant>,
    closed: bool,
    timer: Option<JoinHandle<()>>,
}

impl<O: OrderEvent + EventKind + Sync> State<O> {
    /// Applies the orders and tells whether the sides the listener gets changed.
    fn update(&mut self, source: IndexedEventSource, orders: Vec<O>, is_snapshot: bool) -> bool {
        let limit = self.depth_limit;
        if is_snapshot {
            let source = source.get_id();
            self.by_index.retain(|_, order| order.source() != source);
            self.buy.clear_source(source);
            self.sell.clear_source(source);
        }
        for order in orders {
            if let Some(removed) = self.by_index.remove(&order.index()) {
                match removed.order_side() {
                    Side::Buy => self.buy.remove(&removed, limit),
                    _ => self.sell.remove(&removed, limit),
                }
            }
            if order.has_size() && !order.event_flags().contains(EventFlags::REMOVE_EVENT) {
                let order = Arc::new(order);
                self.by_index.insert(order.index(), Arc::clone(&order));
                match order.order_side() {
                    Side::Buy => self.buy.insert(order, limit),
                    _ => self.sell.insert(order, limit),
                }
            }
        }
        self.buy.changed || self.sell.changed
    }
}

/// One side of the book, sorted from the best price: the buy side when `BUY`, else the sell side.
struct BookSide<O, const BUY: bool> {
    orders: BTreeSet<Ranked<O, BUY>>,
    /// The orders the listener got last, within the depth limit.
    snapshot: Arc<[O]>,
    /// Whether an order within the depth limit changed since the snapshot.
    changed: bool,
}

impl<O, const BUY: bool> Default for BookSide<O, BUY> {
    fn default() -> Self {
        BookSide {
            orders: BTreeSet::new(),
            snapshot: Arc::from(Vec::new()),
            changed: false,
        }
    }
}

impl<O: OrderEvent + EventKind + Sync, const BUY: bool> BookSide<O, BUY> {
    fn insert(&mut self, order: Arc<O>, limit: usize) {
        if self.orders.insert(Ranked(Arc::clone(&order))) {
            self.mark_changed(&order, limit);
        }
    }

    fn remove(&mut self, order: &Arc<O>, limit: usize) {
        if self.orders.remove(&Ranked(Arc::clone(order))) {
            self.mark_changed(order, limit);
        }
    }

    fn clear_source(&mut self, source: i32) {
        let before = self.orders.len();
        self.orders.retain(|order| order.0.source() != source);
        if self.orders.len() != before {
            self.changed = true;
        }
    }

    /// Marks the side changed if the order is within the depth limit.
    fn mark_changed(&mut self, order: &O, limit: usize) {
        if self.changed {
            return;
        }
        let within = match self.snapshot.last() {
            None => true,
            Some(last) => rank::<O, BUY>(order, last) != Ordering::Greater,
        };
        if limit == 0 || self.orders.len() <= limit || within {
            self.changed = true;
        }
    }

    /// The best orders within the depth limit, recomputed if the side changed.
    fn snapshot(&mut self, limit: usize) -> Arc<[O]> {
        if self.changed {
            self.changed = false;
            let limit = if limit == 0 { usize::MAX } else { limit };
            self.snapshot = self.orders.iter().take(limit).map(|order| O::clone(&order.0)).collect();
        }
        Arc::clone(&self.snapshot)
    }
}

/// An order in a side of the book, ordered by [`rank`].
struct Ranked<O, const BUY: bool>(Arc<O>);

impl<O: OrderEvent + EventKind + Sync, const BUY: bool> Ord for Ranked<O, BUY> {
    fn cmp(&self, other: &Self) -> Ordering {
        rank::<O, BUY>(&self.0, &other.0)
    }
}

impl<O: OrderEvent + EventKind + Sync, const BUY: bool> PartialOrd for Ranked<O, BUY> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<O: OrderEvent + EventKind + Sync, const BUY: bool> PartialEq for Ranked<O, BUY> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl<O: OrderEvent + EventKind + Sync, const BUY: bool> Eq for Ranked<O, BUY> {}

/// The order of a side, as the C++ and Java models sort it: the best price first, then [`by_kind`].
fn rank<O: OrderEvent + EventKind + Sync, const BUY: bool>(a: &O, b: &O) -> Ordering {
    let by_price = if BUY {
        b.price().total_cmp(&a.price())
    } else {
        a.price().total_cmp(&b.price())
    };
    by_price.then_with(|| by_kind(a, b))
}

/// The order at one price: the price levels before the individual orders; the larger levels first, then by time,
/// scope, exchange and market maker; the individual orders by time; the index last, which tells all the orders apart.
fn by_kind<O: OrderEvent + EventKind + Sync>(a: &O, b: &O) -> Ordering {
    let individual = |order: &O| order.scope() == Scope::Order;
    match (individual(a), individual(b)) {
        (true, true) => a
            .time_sequence()
            .cmp(&b.time_sequence())
            .then(a.index().cmp(&b.index())),
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => b
            .size()
            .total_cmp(&a.size())
            .then(a.time_sequence().cmp(&b.time_sequence()))
            .then(a.scope().get_code().cmp(&b.scope().get_code()))
            .then(a.exchange_code().cmp(&b.exchange_code()))
            .then_with(|| {
                a.market_maker()
                    .as_deref()
                    .unwrap_or("")
                    .cmp(b.market_maker().as_deref().unwrap_or(""))
            })
            .then(a.index().cmp(&b.index())),
    }
}
