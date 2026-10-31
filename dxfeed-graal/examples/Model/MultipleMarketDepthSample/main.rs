//! Keeps the order books of several symbols with one `MarketDepthModel` per symbol and source, prints each book when it
//! changes and the book of the first symbol and source on Enter, like `MultipleMarketDepthSample` of the other dxFeed APIs. The
//! models work on the default feed, whose address is the `dxfeed.address` system property.
//!
//! Usage: `cargo run --example MultipleMarketDepthSample -- [SYMBOLS] [SOURCES] [ADDRESS]`, by default `AAPL,TSLA` and
//! `NTV`; SYMBOLS and SOURCES are comma-separated lists, and every symbol is taken from every source. Enter prints the
//! first book; `q` or the end of input stops the program. Without ADDRESS the address is `dxfeed.address` of the
//! `dxfeed.system.properties` file in the current directory or of the `DXFEED_dxfeed.address` environment variable,
//! else `demo.dxfeed.com:7300`.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::process::ExitCode;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use dxfeed_graal::event::{IndexedEventSource, Order, OrderSource};
use dxfeed_graal::model::MarketDepthModel;
use dxfeed_graal::{DXEndpoint, DXFeed, system};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let symbols = args.next().unwrap_or_else(|| "AAPL,TSLA".into());
    let sources = args.next().unwrap_or_else(|| "NTV".into());
    match run(&symbols, &sources, args.next()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(symbols: &str, sources: &str, address: Option<String>) -> dxfeed_graal::Result<()> {
    let address = address_or_default(address)?;
    let sources = sources
        .split(',')
        .map(OrderSource::value_of)
        .collect::<dxfeed_graal::Result<Vec<_>>>()?;
    // The default feed connects to `dxfeed.address` when it is first used.
    system::set_property("dxfeed.address", &address)?;

    let model = MultipleMarketDepthModel::new(DXFeed::get_instance()?, 10, Duration::from_secs(10), print_book);
    let mut books: Vec<_> = symbols
        .split(',')
        .flat_map(|symbol| sources.iter().map(move |&source| BookSymbol::new(symbol, source)))
        .collect();
    let mut seen = HashSet::new();
    books.retain(|symbol| seen.insert(symbol.clone()));
    for symbol in &books {
        model.add_symbol(symbol.clone())?;
    }
    println!("Subscribed to {} order books from {address}.", books.len());
    println!(
        "Press Enter to print the {} order book manually, or q and Enter to stop.",
        books[0]
    );

    let mut line = String::new();
    while matches!(std::io::stdin().read_line(&mut line), Ok(n) if n > 0) && line.trim() != "q" {
        line.clear();
        if let Some(book) = model.try_get_book(&books[0]) {
            // The lock keeps the books the listener prints out of the block.
            let _stdout = std::io::stdout().lock();
            println!("{:=^72}", "Print manually");
            print_book(&books[0], &book);
            println!("{:=^72}", "");
        }
    }
    drop(model);
    DXEndpoint::get_instance()?.close_and_await_termination()
}

/// Prints the book: the buy and sell orders side by side.
fn print_book(symbol: &BookSymbol, book: &OrderBook) {
    let mut text = format!("{symbol}:\n");
    for i in 0..book.buy.len().max(book.sell.len()) {
        text += &format!(
            "Buy  {} \t Sell  {}\n",
            describe(book.buy.get(i)),
            describe(book.sell.get(i))
        );
    }
    println!("{text}");
}

/// The source, size and price of an order, or `[None]`.
fn describe(order: Option<&Order>) -> String {
    match order {
        Some(order) => {
            let source = IndexedEventSource::from_id(order.source())
                .map_or_else(|_| order.source().to_string(), |source| source.to_string());
            format!(
                "[Source: {source}, Size: {:8.2}, Price: {:8.4}]",
                order.size, order.price
            )
        }
        None => "[None]".into(),
    }
}

/// A symbol and the source of its orders, such as `AAPL{source=NTV}`.
#[derive(Clone, PartialEq, Eq, Hash)]
struct BookSymbol {
    symbol: String,
    source: IndexedEventSource,
}

impl BookSymbol {
    fn new(symbol: &str, source: IndexedEventSource) -> Self {
        BookSymbol {
            symbol: symbol.into(),
            source,
        }
    }
}

impl fmt::Display for BookSymbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{{source={}}}", self.symbol, self.source)
    }
}

/// The buy and sell orders of a book, best first.
struct OrderBook {
    buy: Vec<Order>,
    sell: Vec<Order>,
}

/// The listener of the books: it gets the symbol and the book whenever the book changes.
type BookListener = dyn Fn(&BookSymbol, &OrderBook) + Send + Sync;

/// The order books of several symbols: one `MarketDepthModel` per symbol and source.
struct MultipleMarketDepthModel {
    feed: &'static DXFeed,
    depth_limit: usize,
    aggregation_period: Duration,
    books: Arc<Books>,
}

/// The models and the latest books, shared with the listeners of the models.
struct Books {
    state: Mutex<State>,
    listener: Box<BookListener>,
}

#[derive(Default)]
struct State {
    models: HashMap<BookSymbol, MarketDepthModel<Order>>,
    books: HashMap<BookSymbol, Arc<OrderBook>>,
}

impl MultipleMarketDepthModel {
    fn new(
        feed: &'static DXFeed,
        depth_limit: usize,
        aggregation_period: Duration,
        listener: impl Fn(&BookSymbol, &OrderBook) + Send + Sync + 'static,
    ) -> Self {
        MultipleMarketDepthModel {
            feed,
            depth_limit,
            aggregation_period,
            books: Arc::new(Books {
                state: Mutex::default(),
                listener: Box::new(listener),
            }),
        }
    }

    /// Starts keeping the book of the symbol; adding it again does nothing.
    fn add_symbol(&self, symbol: BookSymbol) -> dxfeed_graal::Result<()> {
        let mut state = self.books.state.lock().unwrap();
        if state.models.contains_key(&symbol) {
            return Ok(());
        }
        let books = Arc::downgrade(&self.books);
        let book_symbol = symbol.clone();
        let model = MarketDepthModel::<Order>::new_builder()
            .with_feed(self.feed)
            .with_symbol(symbol.symbol.as_str())
            .with_sources([symbol.source])
            .with_depth_limit(self.depth_limit)
            .with_aggregation_period(self.aggregation_period)
            .with_listener(move |buy, sell| on_book(&books, &book_symbol, buy, sell))
            .build()?;
        state.models.insert(symbol, model);
        Ok(())
    }

    /// Stops keeping the book of the symbol and forgets it. Add and remove the symbols on one thread: a book of a
    /// symbol removed and added again on another thread at the same time may come from the removed model.
    #[allow(dead_code)] // The sample keeps all its books.
    fn remove_symbol(&self, symbol: &BookSymbol) {
        let model = {
            let mut state = self.books.state.lock().unwrap();
            state.books.remove(symbol);
            state.models.remove(symbol)
        };
        // Dropped without the lock, as it waits for its listener, which takes the lock.
        drop(model);
    }

    /// The latest book of the symbol, if any.
    fn try_get_book(&self, symbol: &BookSymbol) -> Option<Arc<OrderBook>> {
        self.books.state.lock().unwrap().books.get(symbol).cloned()
    }
}

impl Drop for MultipleMarketDepthModel {
    fn drop(&mut self) {
        // The models are dropped without the lock, as they wait for their listeners, which take the lock.
        let models = std::mem::take(&mut self.books.state.lock().unwrap().models);
        drop(models);
    }
}

/// Keeps the new book of the symbol and passes it to the listener, unless the symbol was removed.
fn on_book(books: &Weak<Books>, symbol: &BookSymbol, buy: &[Order], sell: &[Order]) {
    let Some(books) = books.upgrade() else {
        return;
    };
    let book = {
        let mut state = books.state.lock().unwrap();
        if !state.models.contains_key(symbol) {
            return;
        }
        let book = Arc::new(OrderBook {
            buy: buy.to_vec(),
            sell: sell.to_vec(),
        });
        state.books.insert(symbol.clone(), Arc::clone(&book));
        book
    };
    // Called without the lock, so that the listener may get other books.
    (books.listener)(symbol, &book);
}

/// The ADDRESS argument; without it, `dxfeed.address` of the `dxfeed.system.properties` file or of the
/// `DXFEED_dxfeed.address` environment variable, and the demo feed as the last resort.
fn address_or_default(address: Option<String>) -> dxfeed_graal::Result<String> {
    match address {
        Some(address) => Ok(address),
        None => Ok(system::get_property("dxfeed.address")?.unwrap_or_else(|| "demo.dxfeed.com:7300".into())),
    }
}
