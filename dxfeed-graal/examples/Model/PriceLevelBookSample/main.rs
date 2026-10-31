//! Compiles a price level book (market by price) of a symbol from its individual orders (market by order) and prints
//! the best price levels whenever they change, until Enter is pressed, like `PriceLevelBookSample` of the other dxFeed
//! APIs. The book is built on an `IndexedTxModel` of the default feed, whose address is the `dxfeed.address` system
//! property.
//!
//! Usage: `cargo run --example PriceLevelBookSample -- [SYMBOL] [SOURCE] [ADDRESS]`, by default `MSFT` and `NTV`. Without
//! ADDRESS the address is `dxfeed.address` of the `dxfeed.system.properties` file in the current directory or of the
//! `DXFEED_dxfeed.address` environment variable, else `demo.dxfeed.com:7300`.

use std::collections::{BTreeMap, HashMap};
use std::process::ExitCode;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use dxfeed_graal::event::{EventFlags, IndexedEventSource, Order, OrderEvent, OrderSource, Side};
use dxfeed_graal::model::IndexedTxModel;
use dxfeed_graal::{DXEndpoint, DXFeed, system};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let symbol = args.next().unwrap_or_else(|| "MSFT".into());
    let source = args.next().unwrap_or_else(|| "NTV".into());
    match run(&symbol, &source, args.next()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(symbol: &str, source: &str, address: Option<String>) -> dxfeed_graal::Result<()> {
    let address = address_or_default(address)?;
    let source = OrderSource::value_of(source)?;
    // The default feed connects to `dxfeed.address` when it is first used.
    system::set_property("dxfeed.address", &address)?;

    let book = PriceLevelBook::new(
        DXFeed::get_instance()?,
        symbol,
        source,
        10,
        // Updates are printed at most once a second; snapshots at once.
        Duration::from_secs(1),
        print_levels,
    )?;
    println!("Subscribed to the {source} price levels of {symbol} from {address}; press Enter to stop.");

    // Enter or the end of input stops the program.
    let _ = std::io::stdin().read_line(&mut String::new());
    drop(book);
    DXEndpoint::get_instance()?.close_and_await_termination()
}

/// Prints the price levels as a table: the buy levels on the left, the sell levels on the right.
fn print_levels(buy: &[PriceLevel], sell: &[PriceLevel]) {
    if buy.is_empty() && sell.is_empty() {
        return;
    }
    let mut table = format!("{:=^66}\n{:^31} || {:^31}\n", "", "BID", "ASK");
    table += &format!("{0:^15}|{1:^15} || {0:^15}|{1:^15}\n{2:-^66}\n", "Price", "Size", "");
    for i in 0..buy.len().max(sell.len()) {
        table += &format!("{} || {}\n", row(buy.get(i)), row(sell.get(i)));
    }
    table += &format!("{:=^66}", "");
    println!("{table}");
}

/// A row of the table for a price level, or blanks.
fn row(level: Option<&PriceLevel>) -> String {
    match level {
        Some(level) => format!("{:>14.4} | {:<14.2}", level.price, level.size),
        None => format!("{:>14} | {:<14}", "", ""),
    }
}

/// The orders of one price of a side, summed up.
#[derive(Clone, Debug, PartialEq)]
struct PriceLevel {
    price: f64,
    size: f64,
}

/// The listener of the book: it gets the best buy and sell levels whenever they change.
type LevelListener = dyn FnMut(&[PriceLevel], &[PriceLevel]) + Send;

/// A price level book of one symbol and one source: it sums up the sizes of the orders of each price, keeps the best
/// `depth_limit` levels of each side and passes them to its listener when they change. Snapshots are passed at once,
/// other changes at most once an aggregation period.
struct PriceLevelBook {
    shared: Arc<Shared>,
    timer: Option<JoinHandle<()>>,
    /// Taken by `Drop` before the timer stops, so that no more events arrive.
    model: Option<IndexedTxModel<Order>>,
}

/// What the book shares with the listener of the model and the timer thread.
struct Shared {
    state: Mutex<State>,
    wake: Condvar,
}

struct State {
    /// The orders by their indices, in order, so that the sizes of a level always add up the same.
    orders: BTreeMap<i64, Order>,
    depth_limit: usize,
    aggregation_period: Duration,
    /// When the timer passes the changes on; `None` while nothing waits.
    deadline: Option<Instant>,
    /// The levels the listener got last.
    last: (Vec<PriceLevel>, Vec<PriceLevel>),
    listener: Box<LevelListener>,
    closed: bool,
}

impl PriceLevelBook {
    fn new(
        feed: &DXFeed,
        symbol: &str,
        source: IndexedEventSource,
        depth_limit: usize,
        aggregation_period: Duration,
        listener: impl FnMut(&[PriceLevel], &[PriceLevel]) + Send + 'static,
    ) -> dxfeed_graal::Result<Self> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                orders: BTreeMap::new(),
                depth_limit,
                aggregation_period,
                deadline: None,
                last: Default::default(),
                listener: Box::new(listener),
                closed: false,
            }),
            wake: Condvar::new(),
        });
        let timer = {
            let shared = Arc::clone(&shared);
            thread::spawn(move || shared.run_timer())
        };
        let received = Arc::clone(&shared);
        let model = IndexedTxModel::<Order>::new_builder()
            .with_feed(feed)
            .with_symbol(symbol)
            .with_sources([source])
            .with_listener(move |source, orders, is_snapshot| {
                received.received(source, orders.map(|order| order.to_owned()), is_snapshot)
            })
            .build();
        let mut book = PriceLevelBook {
            shared,
            timer: Some(timer),
            model: None,
        };
        // On an error, dropping the book stops the timer.
        book.model = Some(model?);
        Ok(book)
    }
}

impl Drop for PriceLevelBook {
    fn drop(&mut self) {
        // The model waits for a running call of its listener; then the timer stops.
        drop(self.model.take());
        self.shared.state.lock().unwrap_or_else(PoisonError::into_inner).closed = true;
        self.shared.wake.notify_all();
        if let Some(timer) = self.timer.take() {
            let _ = timer.join();
        }
    }
}

impl Shared {
    /// Applies a transaction or a snapshot of the source and passes the changes on, at once or by the timer.
    fn received(&self, source: IndexedEventSource, orders: impl Iterator<Item = Order>, is_snapshot: bool) {
        let mut state = self.state.lock().unwrap();
        if is_snapshot {
            state.orders.retain(|_, order| order.source() != source.get_id());
        }
        for order in orders {
            state.orders.remove(&order.index);
            if order.has_size() && !order.event_flags.contains(EventFlags::REMOVE_EVENT) {
                state.orders.insert(order.index, order);
            }
        }
        if is_snapshot || state.aggregation_period.is_zero() {
            state.deadline = None;
            notify(state);
        } else if state.deadline.is_none() {
            state.deadline = Some(Instant::now() + state.aggregation_period);
            self.wake.notify_all();
        }
    }

    /// Passes the changes on when their deadline comes, until the book is dropped.
    fn run_timer(&self) {
        let mut state = self.state.lock().unwrap();
        while !state.closed {
            state = match state.deadline {
                Some(deadline) if deadline <= Instant::now() => {
                    state.deadline = None;
                    notify(state);
                    self.state.lock().unwrap()
                }
                Some(deadline) => self.wake.wait_timeout(state, deadline - Instant::now()).unwrap().0,
                None => self.wake.wait(state).unwrap(),
            };
        }
    }
}

/// Passes the best levels to the listener if they changed since its last call.
fn notify(mut state: MutexGuard<'_, State>) {
    let buy = levels(&state, Side::Buy);
    let sell = levels(&state, Side::Sell);
    if state.last.0 != buy || state.last.1 != sell {
        (state.listener)(&buy, &sell);
        state.last = (buy, sell);
    }
}

/// The best levels of the side: buy prices descending, sell prices ascending. Orders without a side are skipped.
fn levels(state: &State, side: Side) -> Vec<PriceLevel> {
    let mut sizes = HashMap::<u64, f64>::new();
    for order in state.orders.values().filter(|order| order.order_side() == side) {
        *sizes.entry(order.price.to_bits()).or_default() += order.size;
    }
    let mut levels: Vec<_> = sizes
        .into_iter()
        .map(|(price, size)| PriceLevel {
            price: f64::from_bits(price),
            size,
        })
        .collect();
    levels.sort_by(|a, b| match side {
        Side::Buy => b.price.total_cmp(&a.price),
        _ => a.price.total_cmp(&b.price),
    });
    if state.depth_limit > 0 {
        levels.truncate(state.depth_limit);
    }
    levels
}

/// The ADDRESS argument; without it, `dxfeed.address` of the `dxfeed.system.properties` file or of the
/// `DXFEED_dxfeed.address` environment variable, and the demo feed as the last resort.
fn address_or_default(address: Option<String>) -> dxfeed_graal::Result<String> {
    match address {
        Some(address) => Ok(address),
        None => Ok(system::get_property("dxfeed.address")?.unwrap_or_else(|| "demo.dxfeed.com:7300".into())),
    }
}
