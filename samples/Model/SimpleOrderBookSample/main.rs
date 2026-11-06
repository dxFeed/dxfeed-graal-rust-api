//! Builds an order book of a symbol and an order source with `IndexedEventTxModel` and prints its top levels every
//! second. Use the default DXFeed instance for that data feed address is defined by "dxfeed.system.properties" file.
//! The properties file is read from the current directory, so run the sample from its directory:
//! `cargo run --bin SimpleOrderBookSample -- <symbol> <source>`.

use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dxfeed_graal::DXFeed;
use dxfeed_graal::event::{EventFlags, Events, IndexedEventSource, Order, OrderEvent, OrderSource, Side};
use dxfeed_graal::model::IndexedEventTxModel;

fn print_usage() {
    let usage_string = r#"
Usage:
SimpleOrderBookSample <symbol> <source>

Where:
    symbol - The security symbol to get events for (e.g. "AAPL").
    source - The order source (e.g. "ntv").

Example:
    SimpleOrderBookSample AAPL ntv"#;

    println!("{usage_string}");
}

struct OrderBook {
    buy_orders: Vec<Arc<Order>>,
    sell_orders: Vec<Arc<Order>>,
}

/// A buy order in a set: the price in descending order, the index if the prices are equal (`BuyOrderComparator`).
struct BuyOrder(Arc<Order>);

impl Ord for BuyOrder {
    fn cmp(&self, other: &Self) -> Ordering {
        // The price in descending order, the index if the prices are equal.
        other
            .0
            .price
            .total_cmp(&self.0.price)
            .then_with(|| self.0.index.cmp(&other.0.index))
    }
}

impl PartialOrd for BuyOrder {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for BuyOrder {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for BuyOrder {}

/// A sell order in a set: the price in ascending order, the index if the prices are equal (`SellOrderComparator`).
struct SellOrder(Arc<Order>);

impl Ord for SellOrder {
    fn cmp(&self, other: &Self) -> Ordering {
        // The price in ascending order, the index if the prices are equal.
        self.0
            .price
            .total_cmp(&other.0.price)
            .then_with(|| self.0.index.cmp(&other.0.index))
    }
}

impl PartialOrd for SellOrder {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for SellOrder {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for SellOrder {}

#[derive(Default)]
struct OrderListener {
    mutex: Mutex<Orders>,
}

/// What the mutex of the listener guards.
#[derive(Default)]
struct Orders {
    orders_by_index: HashMap<i64, Arc<Order>>,
    buy_orders: BTreeSet<BuyOrder>,
    sell_orders: BTreeSet<SellOrder>,
}

impl Orders {
    fn remove(&mut self, order: &Arc<Order>) {
        if order.order_side() == Side::Buy {
            self.buy_orders.remove(&BuyOrder(Arc::clone(order)));
        } else {
            self.sell_orders.remove(&SellOrder(Arc::clone(order)));
        }
    }

    fn add(&mut self, order: &Arc<Order>) {
        if order.order_side() == Side::Buy {
            self.buy_orders.insert(BuyOrder(Arc::clone(order)));
        } else {
            self.sell_orders.insert(SellOrder(Arc::clone(order)));
        }
    }
}

impl OrderListener {
    fn events_received(&self, events: Events<'_, Order>, is_snapshot: bool) {
        let mut orders = self.mutex.lock().unwrap();

        if is_snapshot {
            // if this is a snapshot, clear the order book
            orders.orders_by_index.clear();
            orders.buy_orders.clear();
            orders.sell_orders.clear();
        }

        for order in events {
            let order = Arc::new(order.to_owned());

            // Remove the old order if it exists.
            if let Some(found) = orders.orders_by_index.remove(&order.index) {
                orders.remove(&found);
            }

            if order.event_flags.contains(EventFlags::REMOVE_EVENT) {
                // skip removed events
                continue;
            }

            orders.orders_by_index.insert(order.index, Arc::clone(&order));

            if order.has_size() {
                // add the order to the appropriate side if needed
                orders.add(&order);
            }
        }
    }

    fn get_book(&self) -> OrderBook {
        let orders = self.mutex.lock().unwrap();

        OrderBook {
            buy_orders: orders.buy_orders.iter().map(|order| Arc::clone(&order.0)).collect(),
            sell_orders: orders.sell_orders.iter().map(|order| Arc::clone(&order.0)).collect(),
        }
    }
}

fn get_order_info(orders: &[Arc<Order>], i: usize, side: &Side) -> String {
    let mut info = String::new();

    info += &side.to_string();

    if i < orders.len() {
        let order = &orders[i];

        info += &format!(
            " [Source: {}, Size: {:8.4}, Price: {:8.2}]",
            get_source(order),
            order.size,
            order.price
        );
    } else {
        info += " [None]";
    }

    info
}

fn print_book(book: &OrderBook, depth_limit: usize) {
    if book.buy_orders.is_empty() && book.sell_orders.is_empty() {
        return;
    }

    let limit = book.buy_orders.len().max(book.sell_orders.len()).min(depth_limit);

    for i in 0..limit {
        println!(
            "{}\t{}",
            get_order_info(&book.buy_orders, i, &Side::Buy),
            get_order_info(&book.sell_orders, i, &Side::Sell)
        );
    }

    println!();
}

fn main() -> dxfeed_graal::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 3 {
        print_usage();

        return Ok(());
    }

    let symbol = args[1].as_str();
    let source = OrderSource::value_of(&args[2])?;
    let listener = Arc::new(OrderListener::default());
    let model_listener = Arc::clone(&listener);

    // The endpoint address to use is stored in the "dxfeed.system.properties" file.
    let _model = IndexedEventTxModel::<Order>::new_builder()
        .with_feed(DXFeed::get_instance()?)
        .with_symbol(symbol)
        .with_source(source)
        .with_listener(move |events, is_snapshot| model_listener.events_received(events, is_snapshot))
        .build()?;

    let depth_limit: usize = 5;

    loop {
        print_book(&listener.get_book(), depth_limit); // prints an order book every second
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// The source of an order.
fn get_source(order: &Order) -> IndexedEventSource {
    order.source()
}
