//! Event streams: the events of a subscription as owned events for async code.

use std::collections::VecDeque;
use std::fmt;
use std::future::poll_fn;
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::task::{Context, Poll, Waker};

use crate::endpoint::EndpointState;
use crate::error::{Error, Result};
use crate::event::{Event, SubscriptionItem};
use crate::feed::DXFeed;
use crate::observable::ObservableSubscriptionChangeListener;
use crate::subscription::{DXFeedSubscription, DXFeedTimeSeriesSubscription};
use crate::symbol::Symbol;
use crate::terminate::{self, Terminate};

/// How an [`EventStream`] buffers the events that wait to be taken.
///
/// The default buffers up to [`DEFAULT_CAPACITY`](Self::DEFAULT_CAPACITY) events and drops the oldest ones when more
/// come, so a slow consumer never blocks the threads of the SDK nor grows the memory without limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamOptions {
    capacity: Option<usize>,
}

impl StreamOptions {
    /// The capacity of the default options: 65 536 events.
    pub const DEFAULT_CAPACITY: usize = 65_536;

    /// Buffers up to `capacity` events; when more come, the oldest are dropped and counted in
    /// [`EventStream::dropped`]. A capacity of zero is rejected when the stream is made.
    pub const fn bounded(capacity: usize) -> Self {
        StreamOptions {
            capacity: Some(capacity),
        }
    }

    /// Buffers every event until it is taken: nothing is dropped, and the memory grows while the consumer lags.
    pub const fn unbounded() -> Self {
        StreamOptions { capacity: None }
    }

    /// The capacity, or `None` for [`unbounded`](Self::unbounded).
    pub const fn capacity(&self) -> Option<usize> {
        self.capacity
    }
}

impl Default for StreamOptions {
    fn default() -> Self {
        StreamOptions::bounded(Self::DEFAULT_CAPACITY)
    }
}

/// The events of a subscription as owned events, taken with [`next`](Self::next) in async code or, with the cargo
/// feature `stream`, through the `futures_core::Stream` trait (`StreamExt` of `futures` or `tokio-stream`).
///
/// It is made by [`DXFeedSubscription::into_stream`] or [`DXFeedTimeSeriesSubscription::into_stream`] and owns the
/// subscription, which [`subscription`](Self::subscription) gives for its symbols and settings. It yields `E` for a
/// subscription to one event type, such as [`Quote`](crate::event::Quote), and [`Event`] for an untyped one, which
/// skips the events of types that the crate does not map and counts them in [`skipped`](Self::skipped).
///
/// The events come from a listener on a thread of the endpoint's executor, which copies each batch into the buffer
/// and wakes the task once; it never waits for the consumer. See [`StreamOptions`] for what happens when the buffer
/// is full.
///
/// The stream ends, yielding `None` after the buffered events, when:
/// - it is [closed](Self::close), or its subscription is;
/// - the endpoints of all the feeds that the subscription is attached to are closed, with
///   [`DXEndpoint::close`](crate::DXEndpoint::close) or
///   [`close_and_await_termination`](crate::DXEndpoint::close_and_await_termination);
/// - [`system::shutdown`](crate::system::shutdown) runs.
///
/// After [`DXEndpoint::close`](crate::DXEndpoint::close), the events already scheduled are discarded; after
/// `close_and_await_termination` they are in the buffer. An ended stream stays ended: attaching its subscription to
/// another feed later brings no events. Dropping the stream drops the subscription, which closes it.
///
/// With a [`ManualExecutor`](crate::ManualExecutor), the events come only while the application processes the
/// executor, so a task that awaits the stream on the thread that should process it never wakes.
///
/// # Example
///
/// ```no_run
/// use dxfeed_graal::DXFeed;
/// use dxfeed_graal::event::Quote;
///
/// async fn print_quotes(feed: &DXFeed) -> dxfeed_graal::Result<()> {
///     let mut quotes = feed.create_subscription::<Quote>()?.into_stream()?;
///     quotes.subscription().add_symbols(["AAPL", "IBM"])?;
///     while let Some(quote) = quotes.next().await {
///         println!("{quote}");
///     }
///     Ok(())
/// }
/// ```
#[must_use = "a stream does nothing unless polled; dropping it closes its subscription"]
pub struct EventStream<E: SubscriptionItem = Event, S: AsRef<DXFeedSubscription<E>> = DXFeedSubscription<E>> {
    queue: Arc<Queue<E>>,
    subscription: S,
}

impl<E: SubscriptionItem> DXFeedSubscription<E> {
    /// Turns the subscription into a stream of owned events with the default [`StreamOptions`].
    ///
    /// Make the stream before adding symbols: the SDK refuses a listener of a subscription that has symbols, so this
    /// returns that [`Error::Java`], and the subscription is dropped.
    pub fn into_stream(self) -> Result<EventStream<E>> {
        EventStream::new(self, StreamOptions::default())
    }

    /// Turns the subscription into a stream of owned events with `options`, as [`into_stream`](Self::into_stream)
    /// does.
    ///
    /// Returns [`Error::InvalidArgument`] for a capacity of zero.
    pub fn into_stream_with(self, options: StreamOptions) -> Result<EventStream<E>> {
        EventStream::new(self, options)
    }
}

impl<E: SubscriptionItem> DXFeedTimeSeriesSubscription<E> {
    /// Turns the subscription into a stream of owned events with the default [`StreamOptions`], as
    /// [`DXFeedSubscription::into_stream`] does; [`EventStream::subscription`] keeps
    /// [`set_from_time`](Self::set_from_time).
    pub fn into_stream(self) -> Result<EventStream<E, DXFeedTimeSeriesSubscription<E>>> {
        EventStream::new(self, StreamOptions::default())
    }

    /// Turns the subscription into a stream of owned events with `options`, as
    /// [`DXFeedSubscription::into_stream_with`] does.
    pub fn into_stream_with(self, options: StreamOptions) -> Result<EventStream<E, DXFeedTimeSeriesSubscription<E>>> {
        EventStream::new(self, options)
    }
}

impl<E: SubscriptionItem, S: AsRef<DXFeedSubscription<E>>> EventStream<E, S> {
    fn new(subscription: S, options: StreamOptions) -> Result<Self> {
        if options.capacity == Some(0) {
            return Err(Error::InvalidArgument(
                "the capacity of an event stream must be positive".into(),
            ));
        }
        let base = subscription.as_ref();
        let queue = Arc::new(Queue {
            state: Mutex::new(State {
                items: VecDeque::new(),
                closed: false,
                waker: None,
                dropped: 0,
                skipped: 0,
            }),
            capacity: options.capacity,
            feeds: Arc::downgrade(base.shared_feeds()),
        });
        // Registered first, so that a shutdown that runs meanwhile ends the stream.
        terminate::register(&queue);
        let listener = queue.clone();
        base.add_event_listener(move |events| {
            let mut batch = Vec::with_capacity(events.size_hint().0);
            let mut skipped = 0;
            for item in events {
                match crate::event::owned::<E>(item) {
                    Some(event) => batch.push(event),
                    None => skipped += 1,
                }
            }
            listener.push(batch, skipped);
        })?;
        base.add_change_listener(Closer(queue.clone()))?;
        // A closed subscription does not call the change listener, and an endpoint may have closed before.
        if base.is_closed()? {
            queue.terminate();
        }
        queue.check_feeds(None);
        Ok(EventStream { queue, subscription })
    }

    /// The subscription of the stream, to add symbols or change its settings.
    pub fn subscription(&self) -> &S {
        &self.subscription
    }

    /// The next event, or `None` once the stream has ended and its buffer is empty.
    pub async fn next(&mut self) -> Option<E> {
        poll_fn(|cx| self.poll_next(cx)).await
    }

    /// Takes the next event, or `None` once the stream has ended and its buffer is empty; when the buffer is empty
    /// and the stream goes on, returns [`Poll::Pending`] and wakes the task of `cx` when an event comes or the stream
    /// ends. After `None`, it returns `None` again.
    pub fn poll_next(&mut self, cx: &mut Context<'_>) -> Poll<Option<E>> {
        let replaced = {
            let mut state = self.queue.lock();
            if let Some(event) = state.items.pop_front() {
                return Poll::Ready(Some(event));
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

    /// Ends the stream and closes its subscription. The buffered events are still taken; the events that come later
    /// are discarded. Closing again does nothing.
    pub fn close(&self) -> Result<()> {
        self.queue.terminate();
        self.subscription.as_ref().close()
    }

    /// The number of the events in the buffer.
    pub fn buffered(&self) -> usize {
        self.queue.lock().items.len()
    }

    /// The number of the events dropped so far because the buffer was full.
    pub fn dropped(&self) -> u64 {
        self.queue.lock().dropped
    }

    /// The number of the events skipped so far because the crate does not map their type
    /// ([`EventRef::Other`](crate::EventRef::Other)); always zero for a stream of one event type.
    pub fn skipped(&self) -> u64 {
        self.queue.lock().skipped
    }
}

#[cfg(feature = "stream")]
impl<E: SubscriptionItem, S: AsRef<DXFeedSubscription<E>> + Unpin> futures_core::Stream for EventStream<E, S> {
    type Item = E;

    fn poll_next(self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<E>> {
        EventStream::poll_next(self.get_mut(), cx)
    }

    /// The buffered events, and no upper bound until the stream has ended.
    fn size_hint(&self) -> (usize, Option<usize>) {
        let state = self.queue.lock();
        let buffered = state.items.len();
        (buffered, state.closed.then_some(buffered))
    }
}

impl<E: SubscriptionItem, S: AsRef<DXFeedSubscription<E>>> Drop for EventStream<E, S> {
    fn drop(&mut self) {
        // Ended without a wake, as nothing polls it any more; the subscription is dropped after this with no lock
        // held, as that waits for a running call of the listener.
        let waker = {
            let mut state = self.queue.lock();
            state.closed = true;
            state.items.clear();
            state.waker.take()
        };
        drop(waker);
    }
}

impl<E: SubscriptionItem, S: AsRef<DXFeedSubscription<E>>> fmt::Debug for EventStream<E, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.queue.lock();
        f.debug_struct("EventStream")
            .field("buffered", &state.items.len())
            .field("closed", &state.closed)
            .field("dropped", &state.dropped)
            .field("skipped", &state.skipped)
            .finish()
    }
}

/// The buffer that the listener fills and the stream takes from.
struct Queue<E> {
    state: Mutex<State<E>>,
    capacity: Option<usize>,
    /// The feeds of the subscription, whose endpoints end the stream when all are closed.
    feeds: Weak<Mutex<Vec<DXFeed>>>,
}

struct State<E> {
    items: VecDeque<E>,
    closed: bool,
    waker: Option<Waker>,
    dropped: u64,
    skipped: u64,
}

impl<E> Queue<E> {
    fn lock(&self) -> MutexGuard<'_, State<E>> {
        crate::lock(&self.state)
    }

    /// Adds a batch, dropping the oldest events beyond the capacity, and wakes the task once, after the lock.
    fn push(&self, batch: Vec<E>, skipped: u64) {
        let waker = {
            let mut state = self.lock();
            if state.closed {
                return;
            }
            state.skipped += skipped;
            if batch.is_empty() {
                return;
            }
            state.items.extend(batch);
            if let Some(capacity) = self.capacity
                && state.items.len() > capacity
            {
                let excess = state.items.len() - capacity;
                state.items.drain(..excess);
                state.dropped += excess as u64;
            }
            state.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    /// Ends the stream if the endpoints of all the feeds of the subscription are closed and, when `endpoint` is given,
    /// it is one of them.
    fn check_feeds(&self, endpoint: Option<&EndpointState>) {
        if self.lock().closed {
            return;
        }
        let Some(feeds) = self.feeds.upgrade() else {
            return;
        };
        let ended = {
            let feeds = crate::lock(&feeds);
            !feeds.is_empty()
                && endpoint.is_none_or(|endpoint| feeds.iter().any(|feed| feed.is_of(endpoint)))
                && feeds.iter().all(DXFeed::is_closed)
        };
        if ended {
            self.end();
        }
    }

    /// Ends the stream and wakes the task, after the lock.
    fn end(&self) {
        let waker = {
            let mut state = self.lock();
            state.closed = true;
            state.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

impl<E: Send> Terminate for Queue<E> {
    fn terminate(&self) {
        self.end();
    }

    fn endpoint_closed(&self, endpoint: &EndpointState) {
        self.check_feeds(Some(endpoint));
    }
}

/// The change listener that ends the stream when the subscription is closed.
struct Closer<E>(Arc<Queue<E>>);

impl<E: Send + 'static> ObservableSubscriptionChangeListener for Closer<E> {
    fn symbols_added(&mut self, _symbols: &[Symbol]) {}

    fn subscription_closed(&mut self) {
        self.0.end();
    }
}

impl<E: SubscriptionItem> AsRef<DXFeedSubscription<E>> for DXFeedSubscription<E> {
    fn as_ref(&self) -> &DXFeedSubscription<E> {
        self
    }
}

impl<E: SubscriptionItem> AsRef<DXFeedSubscription<E>> for DXFeedTimeSeriesSubscription<E> {
    fn as_ref(&self) -> &DXFeedSubscription<E> {
        self
    }
}
