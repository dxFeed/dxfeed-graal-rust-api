//! Streams of the models over `LOCAL_HUB` endpoints: whole transactions and books, what a lagging consumer gets, and
//! the end of a stream.

mod support;

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll, Wake, Waker};
use std::time::Duration;

use dxfeed_graal::event::{EventFlags, IndexedEventSource, Order, OrderSource, Scope, Side, TimeAndSale};
use dxfeed_graal::model::{
    BookSides, IndexedTxModel, MarketDepthModel, ModelStream, OrderBookModel, OrderBookModelFilter, TimeSeriesTxModel,
    Transaction,
};
use dxfeed_graal::{Error, live_events, live_handles};
use support::{CountingWaker, Hub, block_on_with};

/// Runs the tests of this file one by one, so that each can count the live handles.
fn serial() -> MutexGuard<'static, ()> {
    static SERIAL: Mutex<()> = Mutex::new(());
    SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
}

const LONG: Duration = Duration::from_secs(10);

type Orders = ModelStream<Transaction<Order>, IndexedTxModel<Order>>;

/// An order of `source` with the index (within the source) and the flags.
fn order_of(source: IndexedEventSource, index: i64, flags: EventFlags) -> Order {
    let mut order = Order {
        index,
        price: 100.0 + index as f64,
        size: 10.0,
        event_flags: flags,
        ..Order::new("TX")
    };
    order.set_source(source).unwrap();
    order.set_order_side(Side::Buy);
    order
}

/// An order of the NTV book.
fn order(index: i64, flags: EventFlags) -> Order {
    order_of(OrderSource::NTV, index, flags)
}

/// The stream of the orders of `TX` from the NTV and ntv books.
fn orders(hub: &Hub) -> Orders {
    IndexedTxModel::<Order>::new_builder()
        .with_feed(&hub.feed)
        .with_symbol("TX")
        .with_sources([OrderSource::NTV, OrderSource::ntv])
        .build_stream()
        .unwrap()
}

fn publish(hub: &Hub, orders: &[Order]) {
    let events: Vec<_> = orders.iter().cloned().map(Into::into).collect();
    hub.publish_and_process(&events);
}

/// Polls `stream` once with `waker`.
fn poll<T, M>(stream: &mut ModelStream<T, M>, waker: &Arc<CountingWaker>) -> Poll<Option<T>> {
    let waker = Waker::from(Arc::clone(waker));
    stream.poll_next(&mut Context::from_waker(&waker))
}

/// The items that `stream` yields until it is pending or ends.
fn take<T, M>(stream: &mut ModelStream<T, M>) -> Vec<T> {
    let waker = CountingWaker::new();
    std::iter::from_fn(|| match poll(stream, &waker) {
        Poll::Ready(item) => item,
        Poll::Pending => None,
    })
    .collect()
}

/// The source, the indices within the source and the snapshot flag of `tx`.
fn shape(tx: &Transaction<Order>) -> (IndexedEventSource, Vec<i64>, bool) {
    let indices = tx.events.iter().map(|order| order.index & 0xFFFF_FFFF).collect();
    (tx.source, indices, tx.is_snapshot)
}

fn snapshot_of(source: IndexedEventSource, indices: &[i64]) -> Vec<Order> {
    let last = indices.len() - 1;
    indices
        .iter()
        .enumerate()
        .map(|(i, &index)| {
            let mut flags = EventFlags::empty();
            if i == 0 {
                flags |= EventFlags::SNAPSHOT_BEGIN;
            }
            if i == last {
                flags |= EventFlags::SNAPSHOT_END;
            }
            order_of(source, index, flags)
        })
        .collect()
}

#[test]
fn streams_are_send_sync_unpin_and_static() {
    fn check<T: Send + Sync + Unpin + 'static>() {}
    check::<Orders>();
    check::<ModelStream<Transaction<TimeAndSale>, TimeSeriesTxModel<TimeAndSale>>>();
}

/// A snapshot and a transaction come as two items, whole, with their source and flag.
#[test]
fn snapshots_and_transactions_come_whole() {
    let _serial = serial();
    let hub = Hub::new();
    let mut stream = orders(&hub);
    hub.executor.process_all_pending().unwrap();
    let waker = CountingWaker::new();
    assert!(poll(&mut stream, &waker).is_pending());

    // Java ends a snapshot with the order of index 0.
    publish(&hub, &snapshot_of(OrderSource::NTV, &[3, 2, 1, 0]));
    assert_eq!(waker.total(), 1);
    publish(
        &hub,
        &[order(4, EventFlags::TX_PENDING), order(6, EventFlags::TX_PENDING)],
    );
    assert_eq!(stream.buffered(), 1, "a pending transaction is not delivered");
    publish(&hub, &[order(5, EventFlags::empty())]);

    let items: Vec<_> = take(&mut stream).iter().map(shape).collect();
    assert_eq!(
        items,
        [
            (OrderSource::NTV, vec![3, 2, 1, 0], true),
            (OrderSource::NTV, vec![4, 6, 5], false),
        ]
    );
    assert_eq!(stream.superseded(), 0);
    assert!(stream.model().is_batch_processing().unwrap());
}

/// A snapshot published in pieces comes as one item once it ends.
#[test]
fn a_snapshot_in_pieces_comes_whole() {
    let _serial = serial();
    let hub = Hub::new();
    let mut stream = orders(&hub);
    hub.executor.process_all_pending().unwrap();
    let snapshot = snapshot_of(OrderSource::NTV, &[3, 2, 1, 0]);
    for piece in snapshot.chunks(1) {
        assert_eq!(stream.buffered(), 0);
        publish(&hub, piece);
    }
    let items: Vec<_> = take(&mut stream).iter().map(shape).collect();
    assert_eq!(items, [(OrderSource::NTV, vec![3, 2, 1, 0], true)]);
}

/// While nothing polls, the transactions of each source wait in order; none is dropped.
#[test]
fn a_lagging_consumer_gets_every_transaction() {
    let _serial = serial();
    let hub = Hub::new();
    let mut stream = orders(&hub);
    hub.executor.process_all_pending().unwrap();
    publish(&hub, &snapshot_of(OrderSource::NTV, &[2, 0]));
    publish(&hub, &snapshot_of(OrderSource::ntv, &[7, 0]));
    publish(&hub, &[order(3, EventFlags::empty())]);
    publish(&hub, &[order_of(OrderSource::ntv, 8, EventFlags::empty())]);
    assert_eq!((stream.buffered(), stream.superseded()), (4, 0));
    let items: Vec<_> = take(&mut stream).iter().map(shape).collect();
    assert_eq!(
        items,
        [
            (OrderSource::NTV, vec![2, 0], true),
            (OrderSource::ntv, vec![7, 0], true),
            (OrderSource::NTV, vec![3], false),
            (OrderSource::ntv, vec![8], false),
        ]
    );
}

/// A time-series stream yields its snapshot; a new time and a new snapshot supersede what is buffered.
#[test]
fn time_series_stream() {
    let _serial = serial();
    let hub = Hub::new();
    let mut stream = TimeSeriesTxModel::<TimeAndSale>::new_builder()
        .with_feed(&hub.feed)
        .with_symbol("TX")
        .with_from_time(1_000)
        .build_stream()
        .unwrap();
    hub.executor.process_all_pending().unwrap();
    assert_eq!(stream.model().get_from_time().unwrap(), 1_000);

    let sale = |time: i64, flags: EventFlags| {
        let mut sale = TimeAndSale {
            price: 10.0,
            size: 1.0,
            event_flags: flags,
            ..TimeAndSale::new("TX")
        };
        sale.set_time(time);
        sale.into()
    };
    hub.publish_and_process(&[
        sale(3_000, EventFlags::SNAPSHOT_BEGIN),
        sale(2_000, EventFlags::empty()),
        sale(1_000, EventFlags::SNAPSHOT_END),
    ]);
    hub.publish_and_process(&[sale(4_000, EventFlags::empty())]);
    assert_eq!(stream.buffered(), 2);

    // A new time subscribes again: the hub sends its history as a new snapshot.
    stream.model().set_from_time(2_000).unwrap();
    hub.executor.process_all_pending().unwrap();
    let items: Vec<(Vec<i64>, bool)> = take(&mut stream)
        .into_iter()
        .map(|tx| (tx.events.iter().map(TimeAndSale::time).collect(), tx.is_snapshot))
        .collect();
    assert_eq!(items, [(vec![4_000, 3_000, 2_000], true)]);
    assert_eq!(stream.superseded(), 2);
}

/// A stream takes the place of the listener; the other errors are those of `build`.
#[test]
fn build_stream_errors() {
    let _serial = serial();
    let hub = Hub::new();
    let result = IndexedTxModel::<Order>::new_builder()
        .with_feed(&hub.feed)
        .with_symbol("TX")
        .with_listener(|_, _, _| {})
        .build_stream();
    assert!(matches!(result, Err(Error::InvalidArgument(_))), "{result:?}");
    let result = TimeSeriesTxModel::<TimeAndSale>::new_builder()
        .with_symbol("TX")
        .with_listener(|_, _, _| {})
        .build_stream();
    assert!(matches!(result, Err(Error::InvalidArgument(_))), "{result:?}");
    let result = IndexedTxModel::<Order>::new_builder()
        .with_feed(&hub.feed)
        .build_stream();
    assert!(result.is_err(), "no symbol: {result:?}");
}

/// A closed stream yields its buffered items, then `None` for good; it closes its model and wakes a waiting task.
#[test]
fn close_drains_then_ends() {
    let _serial = serial();
    let hub = Hub::new();
    let mut stream = orders(&hub);
    hub.executor.process_all_pending().unwrap();
    publish(&hub, &snapshot_of(OrderSource::NTV, &[1, 0]));
    stream.close().unwrap();
    stream.close().unwrap();
    publish(&hub, &[order(2, EventFlags::empty())]);
    let waker = CountingWaker::new();
    assert!(matches!(poll(&mut stream, &waker), Poll::Ready(Some(_))));
    assert!(matches!(poll(&mut stream, &waker), Poll::Ready(None)));
    assert!(matches!(poll(&mut stream, &waker), Poll::Ready(None)));

    let mut stream = orders(&hub);
    assert!(poll(&mut stream, &waker).is_pending());
    stream.close().unwrap();
    assert_eq!(waker.total(), 1);
    assert!(matches!(poll(&mut stream, &waker), Poll::Ready(None)));
}

/// Closing the model through the stream ends the stream.
#[test]
fn closing_the_model_ends_the_stream() {
    let _serial = serial();
    let hub = Hub::with_default_executor();
    let mut stream = orders(&hub);
    stream.model().close().unwrap();
    let result = block_on_with(stream.next(), &CountingWaker::new(), LONG);
    assert!(matches!(result, Some(None)), "{result:?}");
}

/// Closing the endpoint, with or without awaiting termination, ends the stream.
#[test]
fn closing_the_endpoint_ends_the_stream() {
    let _serial = serial();
    for await_termination in [false, true] {
        let hub = Hub::with_default_executor();
        let mut stream = orders(&hub);
        let waker = CountingWaker::new();
        assert!(poll(&mut stream, &waker).is_pending());
        if await_termination {
            hub.endpoint.close_and_await_termination().unwrap();
        } else {
            hub.endpoint.close().unwrap();
        }
        assert_eq!(waker.total(), 1, "await_termination={await_termination}");
        assert!(matches!(poll(&mut stream, &waker), Poll::Ready(None)));
    }
}

/// A model attached to two endpoints ends its stream once both are closed; another endpoint does not matter.
#[test]
fn stream_ends_when_all_its_endpoints_close() {
    let _serial = serial();
    let (first, second, other) = (Hub::new(), Hub::new(), Hub::new());
    let mut stream = orders(&first);
    stream.model().attach(&second.feed).unwrap();
    let waker = CountingWaker::new();
    other.endpoint.close().unwrap();
    first.endpoint.close().unwrap();
    assert!(poll(&mut stream, &waker).is_pending());
    second.endpoint.close().unwrap();
    assert!(matches!(poll(&mut stream, &waker), Poll::Ready(None)));
}

/// The transactions scheduled before `close_and_await_termination` are in the buffer when it returns.
#[test]
fn awaited_close_keeps_scheduled_items() {
    let _serial = serial();
    let hub = Hub::with_default_executor();
    let mut stream = orders(&hub);
    let waker = CountingWaker::new();
    // The model subscribes on the executor; wait for it before publishing.
    let first = snapshot_of(OrderSource::NTV, &[1, 0]);
    let events: Vec<_> = first.iter().cloned().map(Into::into).collect();
    hub.publisher.publish_events(&events).unwrap();
    let result = block_on_with(stream.next(), &waker, LONG);
    assert!(matches!(result, Some(Some(_))), "{result:?}");

    let events: Vec<_> = [order(2, EventFlags::empty())].into_iter().map(Into::into).collect();
    hub.publisher.publish_events(&events).unwrap();
    hub.endpoint.close_and_await_termination().unwrap();
    let items: Vec<_> = take(&mut stream).iter().map(shape).collect();
    assert_eq!(items, [(OrderSource::NTV, vec![2], false)]);
    assert!(matches!(poll(&mut stream, &waker), Poll::Ready(None)));
}

/// A waker that formats the stream, which locks its buffer: it hangs if the stream wakes under that lock.
struct Reentrant(Arc<Mutex<Option<Orders>>>);

impl Wake for Reentrant {
    fn wake(self: Arc<Self>) {
        let stream = self.0.lock().unwrap();
        let _ = format!("{stream:?}");
    }
}

#[test]
fn wakes_outside_the_lock() {
    let _serial = serial();
    let hub = Hub::new();
    let stream = Arc::new(Mutex::new(Some(orders(&hub))));
    hub.executor.process_all_pending().unwrap();
    let waker = Waker::from(Arc::new(Reentrant(stream.clone())));
    let poll_again = || {
        stream
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .poll_next(&mut Context::from_waker(&waker))
    };
    assert!(poll_again().is_pending());
    publish(&hub, &snapshot_of(OrderSource::NTV, &[1, 0]));
    assert!(matches!(poll_again(), Poll::Ready(Some(_))));
    assert!(poll_again().is_pending());
    hub.endpoint.close().unwrap();
    assert!(matches!(poll_again(), Poll::Ready(None)));
}

/// A waker that drops the stream, inside the listener call of the model that wakes it.
struct Dropping(Mutex<Option<Orders>>);

impl Wake for Dropping {
    fn wake(self: Arc<Self>) {
        drop(self.0.lock().unwrap().take());
    }
}

#[test]
fn dropping_the_stream_in_the_wake() {
    let _serial = serial();
    let hub = Hub::new();
    let dropping = Arc::new(Dropping(Mutex::new(Some(orders(&hub)))));
    hub.executor.process_all_pending().unwrap();
    let waker = Waker::from(dropping.clone());
    let polled = dropping
        .0
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .poll_next(&mut Context::from_waker(&waker));
    assert!(polled.is_pending());
    publish(&hub, &snapshot_of(OrderSource::NTV, &[1, 0]));
    assert!(dropping.0.lock().unwrap().is_none());
    publish(&hub, &[order(2, EventFlags::empty())]);
}

/// Dropping a stream lets go of its model, its listener and its items.
#[test]
fn drop_releases_everything() {
    let _serial = serial();
    let hub = Hub::new();
    let (handles, events) = (live_handles(), live_events());
    let mut stream = orders(&hub);
    hub.executor.process_all_pending().unwrap();
    publish(&hub, &snapshot_of(OrderSource::NTV, &[1, 0]));
    publish(&hub, &[order(2, EventFlags::empty())]);
    assert!(matches!(poll(&mut stream, &CountingWaker::new()), Poll::Ready(Some(_))));
    drop(stream);
    hub.executor.process_all_pending().unwrap();
    assert_eq!((live_handles(), live_events()), (handles, events));
}

#[test]
fn debug() {
    let _serial = serial();
    let hub = Hub::new();
    let stream = orders(&hub);
    let text = format!("{stream:?}");
    assert!(text.starts_with("ModelStream { model: IndexedTxModel"), "{text}");
    assert!(text.contains("buffered: 0, closed: false, superseded: 0"), "{text}");
}

type Depth = ModelStream<BookSides<Order>, MarketDepthModel<Order>>;
type Book = ModelStream<BookSides<Order>, OrderBookModel>;

#[test]
fn book_streams_are_send_sync_unpin_and_static() {
    fn check<T: Send + Sync + Unpin + 'static>() {}
    check::<Depth>();
    check::<Book>();
}

/// A price level of the NTV book of `symbol`.
fn level(symbol: &str, index: i64, side: Side, price: f64, flags: EventFlags) -> Order {
    let mut order = Order {
        index,
        price,
        size: 1.0,
        event_flags: flags,
        ..Order::new(symbol)
    };
    order.set_source(OrderSource::NTV).unwrap();
    order.set_order_side(side);
    order.set_scope(Scope::Aggregate);
    order
}

/// A snapshot of three buy and three sell levels of `symbol`.
fn book(symbol: &str) -> Vec<Order> {
    vec![
        level(symbol, 5, Side::Buy, 100.0, EventFlags::SNAPSHOT_BEGIN),
        level(symbol, 4, Side::Buy, 101.0, EventFlags::empty()),
        level(symbol, 3, Side::Sell, 103.0, EventFlags::empty()),
        level(symbol, 2, Side::Sell, 102.0, EventFlags::empty()),
        level(symbol, 1, Side::Buy, 99.0, EventFlags::empty()),
        level(symbol, 0, Side::Sell, 104.0, EventFlags::SNAPSHOT_END),
    ]
}

/// The prices of the buy side and of the sell side.
fn prices(book: &BookSides<Order>) -> (Vec<f64>, Vec<f64>) {
    let prices = |orders: &[Order]| orders.iter().map(|order| order.price).collect();
    (prices(&book.buy), prices(&book.sell))
}

fn depth(hub: &Hub) -> Depth {
    let stream = MarketDepthModel::<Order>::new_builder()
        .with_feed(&hub.feed)
        .with_symbol("DEPTH")
        .with_sources([OrderSource::NTV])
        .build_stream()
        .unwrap();
    hub.executor.process_all_pending().unwrap();
    stream
}

/// The market depth stream yields the sorted sides; a lagging consumer gets the last book only.
#[test]
fn market_depth_stream_keeps_the_last_book() {
    let _serial = serial();
    let hub = Hub::new();
    let mut stream = depth(&hub);
    let waker = CountingWaker::new();
    assert!(poll(&mut stream, &waker).is_pending());
    publish(&hub, &book("DEPTH"));
    assert_eq!(waker.total(), 1);
    let books: Vec<_> = take(&mut stream).iter().map(prices).collect();
    assert_eq!(books, [(vec![101.0, 100.0, 99.0], vec![102.0, 103.0, 104.0])]);

    publish(&hub, &[level("DEPTH", 6, Side::Buy, 101.5, EventFlags::empty())]);
    publish(&hub, &[level("DEPTH", 7, Side::Sell, 101.8, EventFlags::empty())]);
    assert_eq!((stream.buffered(), stream.superseded()), (1, 1));
    let books: Vec<_> = take(&mut stream).iter().map(prices).collect();
    assert_eq!(
        books,
        [(vec![101.5, 101.0, 100.0, 99.0], vec![101.8, 102.0, 103.0, 104.0])]
    );

    // The model changes through the stream.
    stream.model().set_depth_limit(1);
    let books: Vec<_> = take(&mut stream).iter().map(prices).collect();
    assert_eq!(books, [(vec![101.5], vec![101.8])]);
}

#[test]
fn market_depth_build_stream_errors() {
    let _serial = serial();
    let hub = Hub::new();
    let result = MarketDepthModel::<Order>::new_builder()
        .with_feed(&hub.feed)
        .with_symbol("DEPTH")
        .with_listener(|_, _| {})
        .build_stream();
    assert!(matches!(result, Err(Error::InvalidArgument(_))), "{result:?}");
    let result = MarketDepthModel::<Order>::new_builder()
        .with_feed(&hub.feed)
        .build_stream();
    assert!(result.is_err(), "no symbol: {result:?}");
}

/// The order book stream yields the book after each change, filtered as the model is.
#[test]
fn order_book_stream() {
    let _serial = serial();
    let hub = Hub::new();
    let mut stream = OrderBookModel::new().unwrap().into_stream().unwrap();
    stream.model().set_symbol(Some("BOOK")).unwrap();
    stream.model().attach(&hub.feed).unwrap();
    hub.executor.process_all_pending().unwrap();
    take(&mut stream);

    let mut composite = level("BOOK", 9, Side::Buy, 98.0, EventFlags::empty());
    composite.set_source(OrderSource::DEFAULT).unwrap();
    composite.set_scope(Scope::Composite);
    let mut events: Vec<_> = book("BOOK")
        .into_iter()
        .map(|mut order| {
            order.event_flags = EventFlags::empty();
            order
        })
        .collect();
    events.push(composite);
    publish(&hub, &events);
    let books = take(&mut stream);
    // The aggregate levels hide the composite quote.
    assert_eq!(
        books.iter().map(prices).collect::<Vec<_>>(),
        [(vec![101.0, 100.0, 99.0], vec![102.0, 103.0, 104.0])]
    );
    assert_eq!(stream.model().get_filter().unwrap(), OrderBookModelFilter::All);

    // A change through the stream gives a new book: the lot size scales the aggregate levels.
    stream.model().set_lot_size(100).unwrap();
    let books = take(&mut stream);
    let sizes: Vec<f64> = books
        .last()
        .expect("a book")
        .buy
        .iter()
        .map(|order| order.size)
        .collect();
    assert_eq!(sizes, [100.0, 100.0, 100.0], "{books:?}");
}

/// The book streams end when their models close, through the stream or not, and when their endpoints close.
#[test]
fn book_streams_end() {
    let _serial = serial();
    let waker = CountingWaker::new();
    let hub = Hub::new();
    let mut stream = depth(&hub);
    stream.model().close().unwrap();
    assert!(matches!(poll(&mut stream, &waker), Poll::Ready(None)));
    let mut stream = depth(&hub);
    stream.close().unwrap();
    assert!(matches!(poll(&mut stream, &waker), Poll::Ready(None)));

    let mut stream = OrderBookModel::new().unwrap().into_stream().unwrap();
    stream.model().close().unwrap();
    assert!(matches!(poll(&mut stream, &waker), Poll::Ready(None)));

    // The stream of a model closed before ends at once.
    let model = OrderBookModel::new().unwrap();
    model.close().unwrap();
    let mut stream = model.into_stream().unwrap();
    assert!(matches!(poll(&mut stream, &waker), Poll::Ready(None)));

    let mut depth = depth(&hub);
    let mut book = OrderBookModel::new().unwrap().into_stream().unwrap();
    book.model().attach(&hub.feed).unwrap();
    assert!(poll(&mut depth, &waker).is_pending());
    assert!(poll(&mut book, &waker).is_pending());
    hub.endpoint.close().unwrap();
    assert!(matches!(poll(&mut depth, &waker), Poll::Ready(None)));
    assert!(matches!(poll(&mut book, &waker), Poll::Ready(None)));
}

/// An order book stream attached to two endpoints ends once both are closed.
#[test]
fn book_stream_ends_when_all_its_endpoints_close() {
    let _serial = serial();
    let (first, second) = (Hub::new(), Hub::new());
    let mut book = OrderBookModel::new().unwrap().into_stream().unwrap();
    book.model().attach(&first.feed).unwrap();
    book.model().attach(&second.feed).unwrap();
    let waker = CountingWaker::new();
    first.endpoint.close().unwrap();
    assert!(poll(&mut book, &waker).is_pending());
    second.endpoint.close().unwrap();
    assert!(matches!(poll(&mut book, &waker), Poll::Ready(None)));
}

/// A waker that drops a stream, inside the listener call of the model that wakes it.
struct DroppingStream<S>(Mutex<Option<S>>);

impl<S: Send + 'static> Wake for DroppingStream<S> {
    fn wake(self: Arc<Self>) {
        drop(self.0.lock().unwrap().take());
    }
}

/// Polls the stream of `dropping` once with it as the waker.
fn poll_dropping<T, M: Send + 'static>(dropping: &Arc<DroppingStream<ModelStream<T, M>>>) -> bool
where
    T: Send + 'static,
{
    let waker = Waker::from(dropping.clone());
    let polled = dropping
        .0
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .poll_next(&mut Context::from_waker(&waker));
    polled.is_pending()
}

#[test]
fn dropping_a_book_stream_in_the_wake() {
    let _serial = serial();
    let hub = Hub::new();
    let dropping = Arc::new(DroppingStream(Mutex::new(Some(depth(&hub)))));
    assert!(poll_dropping(&dropping));
    publish(&hub, &book("DEPTH"));
    assert!(dropping.0.lock().unwrap().is_none());
    publish(&hub, &[level("DEPTH", 6, Side::Buy, 101.5, EventFlags::empty())]);

    let stream = OrderBookModel::new().unwrap().into_stream().unwrap();
    stream.model().set_symbol(Some("BOOK")).unwrap();
    stream.model().attach(&hub.feed).unwrap();
    hub.executor.process_all_pending().unwrap();
    let dropping = Arc::new(DroppingStream(Mutex::new(Some(stream))));
    assert!(poll_dropping(&dropping));
    publish(&hub, &book("BOOK"));
    assert!(dropping.0.lock().unwrap().is_none());
    publish(&hub, &[level("BOOK", 6, Side::Buy, 101.5, EventFlags::empty())]);
}

#[cfg(feature = "stream")]
#[test]
fn stream_trait() {
    use std::pin::Pin;

    use futures_core::Stream;

    let _serial = serial();
    let hub = Hub::new();
    let mut stream = orders(&hub);
    hub.executor.process_all_pending().unwrap();
    assert_eq!(Stream::size_hint(&stream), (0, None));
    publish(&hub, &snapshot_of(OrderSource::NTV, &[1, 0]));
    assert_eq!(Stream::size_hint(&stream), (1, None));
    let waker = Waker::from(CountingWaker::new());
    let mut cx = Context::from_waker(&waker);
    assert!(matches!(
        Stream::poll_next(Pin::new(&mut stream), &mut cx),
        Poll::Ready(Some(_))
    ));
    stream.close().unwrap();
    assert_eq!(Stream::size_hint(&stream), (0, Some(0)));
    assert!(matches!(
        Stream::poll_next(Pin::new(&mut stream), &mut cx),
        Poll::Ready(None)
    ));
}
