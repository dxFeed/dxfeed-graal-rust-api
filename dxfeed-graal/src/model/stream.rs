//! Streams of the models: whole transactions and order books as owned items for async code.

use std::collections::VecDeque;
use std::fmt;
use std::future::poll_fn;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
use std::task::{Context, Poll, Waker};

use crate::endpoint::EndpointState;
use crate::error::Result;
use crate::event::IndexedEventSource;
use crate::feed::DXFeed;
use crate::stream::feeds_of;
use crate::terminate::{self, Terminate};

/// What one call of the listener of a transactional model gets, as owned events: the events of one or, with batch
/// processing, several whole transactions of one source, or a snapshot of that source.
///
/// A snapshot replaces the events of its source that came before; a [`ModelStream`] therefore drops the transactions
/// of that source still in its buffer when a snapshot comes, and counts them in
/// [`superseded`](ModelStream::superseded).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Transaction<E> {
    /// The source of the events.
    pub source: IndexedEventSource,
    /// The events, in the order the model gives them.
    pub events: Vec<E>,
    /// Whether the events are a snapshot, which replaces the earlier events of the source.
    pub is_snapshot: bool,
}

/// The order book as a [`MarketDepthModel`](super::MarketDepthModel) or an [`OrderBookModel`](super::OrderBookModel)
/// shows it after a change: the buy orders from the best (highest) price down and the sell orders from the best
/// (lowest) price up.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct BookSides<O> {
    /// The buy side, the bids.
    pub buy: Arc<[O]>,
    /// The sell side, the offers.
    pub sell: Arc<[O]>,
}

/// The items of a model as owned values, taken with [`next`](Self::next) in async code or, with the cargo feature
/// `stream`, through the `futures_core::Stream` trait (`StreamExt` of `futures` or `tokio-stream`).
///
/// It is made in place of the listener of a model and owns the model, which [`model`](Self::model) gives for its
/// settings:
/// - [`IndexedTxModelBuilder::build_stream`](super::IndexedTxModelBuilder::build_stream) and
///   [`TimeSeriesTxModelBuilder::build_stream`](super::TimeSeriesTxModelBuilder::build_stream) yield a
///   [`Transaction`] per call of the model's listener, whole: a transaction or a snapshot is never split;
/// - [`MarketDepthModelBuilder::build_stream`](super::MarketDepthModelBuilder::build_stream) and
///   [`OrderBookModel::into_stream`](super::OrderBookModel::into_stream) yield [`BookSides`], the whole book after a
///   change.
///
/// The listener copies the items into the buffer of the stream on the thread that calls it (the executor of the model,
/// or for a market depth model also its timer thread and the thread that changes its depth limit) and never waits for
/// the consumer. When the consumer lags, the stream keeps what it needs for the current state: a snapshot drops the
/// buffered transactions of its source, and a book replaces the buffered book; both are counted in
/// [`superseded`](Self::superseded). Otherwise the buffer of transactions grows until the consumer takes them.
///
/// The stream ends, yielding `None` after the buffered items, when:
/// - it is [closed](Self::close), or its model is;
/// - the endpoints of all the feeds that the model is attached to are closed, with
///   [`DXEndpoint::close`](crate::DXEndpoint::close) or
///   [`close_and_await_termination`](crate::DXEndpoint::close_and_await_termination);
/// - [`system::shutdown`](crate::system::shutdown) runs.
///
/// Dropping the stream drops the model, which closes it. With a [`ManualExecutor`](crate::ManualExecutor), the items
/// from the events come only while the application processes the executor, so a task that awaits them on the thread
/// that should process it never wakes.
///
/// # Example
///
/// ```no_run
/// use dxfeed_graal::DXFeed;
/// use dxfeed_graal::event::{Order, OrderSource};
/// use dxfeed_graal::model::IndexedTxModel;
///
/// async fn print_orders(feed: &DXFeed) -> dxfeed_graal::Result<()> {
///     let mut orders = IndexedTxModel::<Order>::new_builder()
///         .with_feed(feed)
///         .with_symbol("AAPL")
///         .with_sources([OrderSource::NTV])
///         .build_stream()?;
///     while let Some(tx) = orders.next().await {
///         let kind = if tx.is_snapshot { "snapshot" } else { "update" };
///         println!("{kind} from {}: {} orders", tx.source, tx.events.len());
///     }
///     Ok(())
/// }
/// ```
#[must_use = "a stream does nothing unless polled; dropping it closes its model"]
pub struct ModelStream<T, M> {
    buffer: Arc<Buffer<T>>,
    close: fn(&M) -> Result<()>,
    model: M,
}

impl<T: Send + 'static, M> ModelStream<T, M> {
    /// The stream of `model`, whose listener fills `buffer`; it watches the endpoints of `feeds`, the feeds of the
    /// model, and `close` closes the model.
    pub(crate) fn new(
        buffer: Arc<Buffer<T>>,
        model: M,
        close: fn(&M) -> Result<()>,
        feeds: &Arc<Mutex<Vec<DXFeed>>>,
    ) -> Self {
        buffer.watch(feeds);
        ModelStream { buffer, close, model }
    }
}

impl<T, M> ModelStream<T, M> {
    /// The model of the stream, to change its settings.
    pub fn model(&self) -> &M {
        &self.model
    }

    /// The next item, or `None` once the stream has ended and its buffer is empty.
    pub async fn next(&mut self) -> Option<T> {
        poll_fn(|cx| self.poll_next(cx)).await
    }

    /// Takes the next item, or `None` once the stream has ended and its buffer is empty; when the buffer is empty and
    /// the stream goes on, returns [`Poll::Pending`] and wakes the task of `cx` when an item comes or the stream ends.
    /// After `None`, it returns `None` again.
    pub fn poll_next(&mut self, cx: &mut Context<'_>) -> Poll<Option<T>> {
        let replaced = {
            let mut state = self.buffer.lock();
            if let Some(item) = state.items.pop_front() {
                return Poll::Ready(Some(item));
            }
            if state.closed {
                return Poll::Ready(None);
            }
            match &mut state.waker {
                Some(waker) if waker.will_wake(cx.waker()) => None,
                waker => waker.replace(cx.waker().clone()),
            }
        };
        // The waker of another task is dropped after the lock, as its drop may run any code.
        drop(replaced);
        Poll::Pending
    }

    /// Ends the stream and closes its model. The buffered items are still taken; the items that come later are
    /// discarded. Closing again does nothing.
    pub fn close(&self) -> Result<()> {
        let closed = (self.close)(&self.model);
        wake(self.buffer.end());
        closed
    }

    /// The number of the items in the buffer.
    pub fn buffered(&self) -> usize {
        self.buffer.lock().items.len()
    }

    /// The number of the items dropped so far from the buffer because a later item replaced them: the transactions of
    /// a source before a snapshot of it, or a book before a later book.
    pub fn superseded(&self) -> u64 {
        self.buffer.lock().superseded
    }
}

#[cfg(feature = "stream")]
impl<T, M: Unpin> futures_core::Stream for ModelStream<T, M> {
    type Item = T;

    fn poll_next(self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<T>> {
        ModelStream::poll_next(self.get_mut(), cx)
    }

    /// The buffered items, and no upper bound until the stream has ended.
    fn size_hint(&self) -> (usize, Option<usize>) {
        let state = self.buffer.lock();
        let buffered = state.items.len();
        (buffered, state.closed.then_some(buffered))
    }
}

impl<T, M> Drop for ModelStream<T, M> {
    fn drop(&mut self) {
        // Ended without a wake, as nothing polls it any more; the model is dropped after this with no lock held, as
        // that waits for a running call of its listener, which may be adding an item.
        let (waker, items) = {
            let mut state = self.buffer.lock();
            state.closed = true;
            (state.waker.take(), std::mem::take(&mut state.items))
        };
        drop(waker);
        drop(items);
    }
}

impl<T, M: fmt::Debug> fmt::Debug for ModelStream<T, M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (buffered, closed, superseded) = {
            let state = self.buffer.lock();
            (state.items.len(), state.closed, state.superseded)
        };
        f.debug_struct("ModelStream")
            .field("model", &self.model)
            .field("buffered", &buffered)
            .field("closed", &closed)
            .field("superseded", &superseded)
            .finish()
    }
}

/// The buffer that the listener of a model fills and its stream takes from.
pub(crate) struct Buffer<T> {
    state: Mutex<State<T>>,
    /// Adds an item to the items and returns those it replaces.
    add: fn(&mut VecDeque<T>, T) -> Vec<T>,
    /// The feeds of the model, whose endpoints end the stream when all are closed; set once the model is built.
    feeds: OnceLock<Weak<Mutex<Vec<DXFeed>>>>,
}

struct State<T> {
    items: VecDeque<T>,
    closed: bool,
    waker: Option<Waker>,
    superseded: u64,
}

impl<E: Send + 'static> Buffer<Transaction<E>> {
    /// A buffer of transactions, registered for shutdown.
    pub(crate) fn of_transactions() -> Arc<Self> {
        Buffer::register(add_transaction)
    }
}

impl<O: Send + Sync + 'static> Buffer<BookSides<O>> {
    /// A buffer of the last book, registered for shutdown.
    pub(crate) fn of_books() -> Arc<Self> {
        Buffer::register(add_book)
    }
}

impl<T: Send + 'static> Buffer<T> {
    fn register(add: fn(&mut VecDeque<T>, T) -> Vec<T>) -> Arc<Self> {
        let buffer = Arc::new(Buffer {
            state: Mutex::new(State {
                items: VecDeque::new(),
                closed: false,
                waker: None,
                superseded: 0,
            }),
            add,
            feeds: OnceLock::new(),
        });
        // Registered before the model is built, so that a shutdown that runs meanwhile ends the stream.
        terminate::register(&buffer);
        buffer
    }

    /// Adds an item from the listener of the model and wakes the task after the lock.
    pub(crate) fn push(&self, item: T) {
        let (waker, superseded) = {
            let mut state = self.lock();
            if state.closed {
                return;
            }
            let superseded = (self.add)(&mut state.items, item);
            state.superseded += superseded.len() as u64;
            (state.waker.take(), superseded)
        };
        // The superseded items are freed after the lock, so that the consumer does not wait for it.
        drop(superseded);
        wake(waker);
    }

    /// Watches the endpoints of `feeds`, and ends the stream if they are all closed already.
    fn watch(&self, feeds: &Arc<Mutex<Vec<DXFeed>>>) {
        let _ = self.feeds.set(Arc::downgrade(feeds));
        self.check_feeds(None);
    }

    /// Ends the stream if the endpoints of all the feeds of the model are closed and, when `endpoint` is given, it is
    /// one of them.
    fn check_feeds(&self, endpoint: Option<&EndpointState>) {
        if let Some(feeds) = self.feeds.get()
            && feeds_of(feeds, endpoint).1
        {
            wake(self.end());
        }
    }
}

impl<T> Buffer<T> {
    fn lock(&self) -> MutexGuard<'_, State<T>> {
        crate::lock(&self.state)
    }

    /// Ends the stream and returns the waker of the task to wake after the lock.
    fn end(&self) -> Option<Waker> {
        let mut state = self.lock();
        state.closed = true;
        state.waker.take()
    }
}

impl<T: Send + 'static> Terminate for Buffer<T> {
    fn terminate(&self) {
        wake(self.end());
    }

    fn endpoint_closed(&self, endpoint: &EndpointState) {
        self.check_feeds(Some(endpoint));
    }
}

/// Adds a transaction; a snapshot takes out the transactions of its source before it.
fn add_transaction<E>(items: &mut VecDeque<Transaction<E>>, transaction: Transaction<E>) -> Vec<Transaction<E>> {
    let mut superseded = Vec::new();
    if transaction.is_snapshot {
        for earlier in std::mem::take(items) {
            if earlier.source == transaction.source {
                superseded.push(earlier);
            } else {
                items.push_back(earlier);
            }
        }
    }
    items.push_back(transaction);
    superseded
}

/// Adds a book in place of the book in the buffer, if any, which it takes out.
fn add_book<O>(items: &mut VecDeque<BookSides<O>>, book: BookSides<O>) -> Vec<BookSides<O>> {
    let superseded = items.drain(..).collect();
    items.push_back(book);
    superseded
}

/// Ends the stream of a model when the model is closed: the model calls it from its `close`.
#[derive(Default)]
pub(crate) struct CloseHook(Mutex<Option<Weak<dyn Terminate>>>);

impl CloseHook {
    /// Makes the close of the model end `buffer`.
    pub(crate) fn set<T: Send + 'static>(&self, buffer: &Arc<Buffer<T>>) {
        let weak: Weak<dyn Terminate> = Arc::downgrade(buffer) as Weak<Buffer<T>>;
        *crate::lock(&self.0) = Some(weak);
    }

    /// Ends the stream of the model, if any.
    pub(crate) fn fire(&self) {
        let hook = crate::lock(&self.0).clone();
        if let Some(buffer) = hook.and_then(|hook| hook.upgrade()) {
            buffer.terminate();
        }
    }
}

/// Wakes the task of `waker`, if any.
fn wake(waker: Option<Waker>) {
    if let Some(waker) = waker {
        waker.wake();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::OrderSource;

    fn tx(source: IndexedEventSource, event: i32, is_snapshot: bool) -> Transaction<i32> {
        Transaction {
            source,
            events: vec![event],
            is_snapshot,
        }
    }

    #[test]
    fn a_snapshot_drops_the_transactions_of_its_source_only() {
        let (a, b) = (OrderSource::NTV, OrderSource::ntv);
        let mut items = VecDeque::new();
        for item in [tx(a, 1, true), tx(b, 2, true), tx(a, 3, false), tx(b, 4, false)] {
            assert_eq!(add_transaction(&mut items, item).len(), 0);
        }
        assert_eq!(add_transaction(&mut items, tx(a, 5, false)).len(), 0, "not a snapshot");
        let superseded: Vec<i32> = add_transaction(&mut items, tx(a, 6, true))
            .iter()
            .map(|tx| tx.events[0])
            .collect();
        assert_eq!(superseded, [1, 3, 5]);
        let events: Vec<i32> = items.iter().map(|tx| tx.events[0]).collect();
        assert_eq!(events, [2, 4, 6]);
    }

    #[test]
    fn a_book_replaces_the_book_in_the_buffer() {
        let book = |n: i32| BookSides::<i32> {
            buy: Arc::from([n]),
            sell: Arc::from([]),
        };
        let mut items = VecDeque::new();
        assert_eq!(add_book(&mut items, book(1)).len(), 0);
        assert_eq!(add_book(&mut items, book(2)).len(), 1);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].buy[0], 2);
    }
}
