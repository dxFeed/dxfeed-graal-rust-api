//! Event streams: the events of a subscription as owned events for async code.

use std::collections::VecDeque;
use std::fmt;
use std::future::poll_fn;
use std::panic::{self, AssertUnwindSafe};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, Weak};
use std::task::{Context, Poll, Waker};
use std::thread;
use std::time::Duration;

use crate::endpoint::EndpointState;
use crate::error::{Error, Result};
use crate::error_manager::{self, ErrorManager};
use crate::event::{Event, SubscriptionItem};
use crate::event_queue::{EventQueue, PollResult};
use crate::feed::DXFeed;
use crate::observable::ObservableSubscriptionChangeListener;
use crate::subscription::{DXFeedSubscription, DXFeedTimeSeriesSubscription};
use crate::symbol::Symbol;
use crate::terminate::{self, Terminate};

/// How an [`EventStream`] gets the events of its subscription and buffers those that wait to be taken.
///
/// There are two ways:
/// - **A listener** (the default, [`bounded`](Self::bounded), [`unbounded`](Self::unbounded)): a listener on a thread
///   of the endpoint's executor copies every event into the buffer of the stream as it comes and never waits for the
///   consumer. The default buffers up to [`DEFAULT_CAPACITY`](Self::DEFAULT_CAPACITY) events and drops the oldest ones
///   when more come, of whatever symbol, so a slow consumer never blocks the threads of the SDK nor grows the memory
///   without limit. The subscription can conflate the events before the listener with
///   [`set_aggregation_period`](DXFeedSubscription::set_aggregation_period).
/// - **The event queue of the subscription** ([`event_queue`](Self::event_queue)): the events stay in the SDK until
///   the consumer waits for them. Meanwhile the SDK conflates the lasting events, such as quotes, so that a lagging
///   consumer gets the last event of each symbol; it keeps the other events, such as trades, in a buffer of the
///   subscription, and an endpoint of [`Role::StreamFeed`](crate::Role::StreamFeed) stops receiving data instead of
///   dropping it. See the [`event_queue`](crate::event_queue) module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamOptions {
    capacity: Option<usize>,
    event_queue: bool,
}

impl StreamOptions {
    /// The capacity of the default options: 65 536 events.
    pub const DEFAULT_CAPACITY: usize = 65_536;

    /// Buffers up to `capacity` events; when more come, the oldest are dropped and counted in
    /// [`EventStream::dropped`]. A capacity of zero is rejected when the stream is made.
    pub const fn bounded(capacity: usize) -> Self {
        StreamOptions {
            capacity: Some(capacity),
            event_queue: false,
        }
    }

    /// Buffers every event until it is taken: nothing is dropped, and the memory grows while the consumer lags.
    pub const fn unbounded() -> Self {
        StreamOptions {
            capacity: None,
            event_queue: false,
        }
    }

    /// Takes the events from an [`EventQueue`] of the subscription, only while the consumer waits for them, so that
    /// the SDK conflates the lasting events meanwhile.
    ///
    /// A thread of the stream polls the queue when the buffer of the stream is empty and a task waits, and moves at
    /// most about 1 024 events into the buffer at a time; the rest stay in the SDK. [`EventStream::dropped`] counts the
    /// events that the SDK lost (`getDroppedEvents` of the queue), including those that it held when the stream was
    /// closed. Each stream has its own thread: prefer one stream for many symbols to a stream per symbol.
    ///
    /// The subscription must have no executor and no other queue, as for [`EventQueue::new`].
    /// [`DXEndpoint::close_and_await_termination`](crate::DXEndpoint::close_and_await_termination) does not wait for
    /// the consumer: the stream takes the events that the queue holds into its buffer before the endpoint waits for
    /// them, without conflating them.
    ///
    /// ```no_run
    /// use dxfeed_graal::event::Quote;
    /// use dxfeed_graal::{DXFeed, StreamOptions};
    ///
    /// async fn print_quotes(feed: &DXFeed) -> dxfeed_graal::Result<()> {
    ///     let subscription = feed.create_subscription::<Quote>()?;
    ///     let mut quotes = subscription.into_stream_with(StreamOptions::event_queue())?;
    ///     quotes.subscription().add_symbols(["AAPL", "IBM"])?;
    ///     while let Some(quote) = quotes.next().await {
    ///         // While this task is busy, the SDK keeps only the last quote of each symbol.
    ///         println!("{quote}");
    ///     }
    ///     Ok(())
    /// }
    /// ```
    pub const fn event_queue() -> Self {
        StreamOptions {
            capacity: None,
            event_queue: true,
        }
    }

    /// The capacity, or `None` for [`unbounded`](Self::unbounded) and [`event_queue`](Self::event_queue).
    pub const fn capacity(&self) -> Option<usize> {
        self.capacity
    }
}

impl Default for StreamOptions {
    fn default() -> Self {
        StreamOptions::bounded(Self::DEFAULT_CAPACITY)
    }
}

/// The timeout of a poll that drains the event queue of a stream for an endpoint that awaits its termination.
const DRAIN_POLL: Duration = Duration::from_millis(10);

/// The limit of a poll of the event queue of a stream.
const BATCH: usize = 1024;

/// The events of a subscription as owned events, taken with [`next`](Self::next) in async code or, with the cargo
/// feature `stream`, through the `futures_core::Stream` trait (`StreamExt` of `futures` or `tokio-stream`).
///
/// It is made by [`DXFeedSubscription::into_stream`] or [`DXFeedTimeSeriesSubscription::into_stream`] and owns the
/// subscription, which [`subscription`](Self::subscription) gives for its symbols and settings. It yields `E` for a
/// subscription to one event type, such as [`Quote`](crate::event::Quote), and [`Event`] for an untyped one, which
/// skips the events of types that the crate does not map and counts them in [`skipped`](Self::skipped).
///
/// By default the events come from a listener on a thread of the endpoint's executor, which copies each batch into
/// the buffer and wakes the task once; it never waits for the consumer. With [`StreamOptions::event_queue`] they stay
/// in the SDK, conflated, until the consumer waits for them. See [`StreamOptions`] for both ways and for what happens
/// when the consumer lags.
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
/// With a [`ManualExecutor`](crate::ManualExecutor), the events of a listener come only while the application
/// processes the executor, so a task that awaits the stream on the thread that should process it never wakes.
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
    /// Returns [`Error::InvalidArgument`] for a capacity of zero. With [`StreamOptions::event_queue`], a subscription
    /// with symbols on a feed, an executor or a queue is [`Error::Java`] of `java.lang.IllegalStateException`, and a
    /// thread that cannot be started is [`Error::Io`].
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
        let source = if options.event_queue {
            Some(EventQueue::new(base)?)
        } else {
            None
        };
        let queue = Arc::new(Queue {
            state: Mutex::new(State {
                items: VecDeque::new(),
                closed: false,
                waker: None,
                dropped: 0,
                skipped: 0,
                demand: false,
                draining: Vec::new(),
                finishing: false,
            }),
            capacity: options.capacity,
            feeds: Arc::downgrade(base.shared_feeds()),
            source,
            requests: Condvar::new(),
        });
        // Registered first, so that a shutdown that runs meanwhile ends the stream.
        terminate::register(&queue);
        if queue.source.is_none() {
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
                wake(listener.push(batch, skipped, None));
            })?;
        }
        base.add_change_listener(Closer(queue.clone()))?;
        // A closed subscription does not call the change listener, and an endpoint may have closed before.
        if base.is_closed()? {
            queue.terminate();
        }
        queue.check_feeds(None);
        if queue.source.is_some() {
            let helper = queue.clone();
            thread::Builder::new()
                .name("dxfeed-event-stream".into())
                .spawn(move || helper.run())
                .map_err(|e| Error::Io(Arc::new(e)))?;
        }
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
        let (replaced, requested) = {
            let mut state = self.queue.lock();
            if let Some(event) = state.items.pop_front() {
                return Poll::Ready(Some(event));
            }
            if state.closed {
                return Poll::Ready(None);
            }
            let requested = self.queue.source.is_some() && !std::mem::replace(&mut state.demand, true);
            let replaced = match &mut state.waker {
                Some(waker) if waker.will_wake(cx.waker()) => None,
                waker => waker.replace(cx.waker().clone()),
            };
            (replaced, requested)
        };
        if requested {
            self.queue.requests.notify_one();
        }
        // The waker of another task is dropped after the lock, as its drop may run any code.
        drop(replaced);
        Poll::Pending
    }

    /// Ends the stream and closes its subscription. The buffered events are still taken; the events that come later
    /// are discarded, and those that the event queue of the stream holds are dropped and counted in
    /// [`dropped`](Self::dropped). Closing again does nothing.
    pub fn close(&self) -> Result<()> {
        let closed = match &self.queue.source {
            Some(source) => {
                let closed = source.close();
                wake(self.queue.end_with(Some(source.get_dropped_events())));
                // Lets the thread of the stream exit now rather than when the stream is dropped.
                self.queue.requests.notify_one();
                closed
            }
            None => {
                self.queue.terminate();
                Ok(())
            }
        };
        closed.and(self.subscription.as_ref().close())
    }

    /// The number of the events in the buffer.
    pub fn buffered(&self) -> usize {
        self.queue.lock().items.len()
    }

    /// The number of the events dropped so far: because the buffer was full, or, with
    /// [`StreamOptions::event_queue`], by the SDK.
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
        if let Some(source) = &self.queue.source {
            // The thread of the stream exits: at once if it waits for a request, after its poll otherwise, which the
            // close ends. It is not joined, as this may run on it, in its wake.
            self.queue.requests.notify_one();
            let _ = source.close();
        }
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
            .field("event_queue", &self.queue.source.is_some())
            .finish()
    }
}

/// The buffer that the listener or the thread of the stream fills and the stream takes from.
struct Queue<E: SubscriptionItem> {
    state: Mutex<State<E>>,
    capacity: Option<usize>,
    /// The feeds of the subscription, whose endpoints end the stream when all are closed.
    feeds: Weak<Mutex<Vec<DXFeed>>>,
    /// The event queue that the thread of the stream polls, for [`StreamOptions::event_queue`].
    source: Option<EventQueue<E>>,
    /// Wakes the thread of the stream when the state asks it to poll or to exit.
    requests: Condvar,
}

struct State<E> {
    items: VecDeque<E>,
    closed: bool,
    waker: Option<Waker>,
    dropped: u64,
    skipped: u64,
    /// A task waits for events: the thread of the stream polls the event queue.
    demand: bool,
    /// The endpoints of the subscription that wait for their termination, by address: while there is one, the thread
    /// polls the event queue with no demand, so that they do not wait for the events that the queue holds.
    draining: Vec<usize>,
    /// The stream ends after the thread has taken what the event queue still holds.
    finishing: bool,
}

impl<E: SubscriptionItem> Queue<E> {
    fn lock(&self) -> MutexGuard<'_, State<E>> {
        crate::lock(&self.state)
    }

    /// Adds a batch, dropping the oldest events beyond the capacity, and returns the waker of the task to wake after
    /// the lock. `lost` is the dropped count of the event queue, if it was read.
    fn push(&self, batch: Vec<E>, skipped: u64, lost: Option<Result<i64>>) -> Option<Waker> {
        let mut state = self.lock();
        if state.closed {
            return None;
        }
        state.skipped += skipped;
        Self::update_lost(&mut state, lost);
        if batch.is_empty() {
            return None;
        }
        state.items.extend(batch);
        if let Some(capacity) = self.capacity
            && state.items.len() > capacity
        {
            let excess = state.items.len() - capacity;
            state.items.drain(..excess);
            state.dropped += excess as u64;
        }
        state.demand = false;
        state.waker.take()
    }

    /// Ends the stream and returns the waker of the task to wake after the lock. `lost` is the dropped count of the
    /// event queue, if it was read.
    fn end_with(&self, lost: Option<Result<i64>>) -> Option<Waker> {
        let mut state = self.lock();
        Self::update_lost(&mut state, lost);
        state.closed = true;
        state.waker.take()
    }

    /// Stores the dropped count of the event queue, which never decreases; one that cannot be read, such as after
    /// shutdown, keeps the last one.
    fn update_lost(state: &mut State<E>, lost: Option<Result<i64>>) {
        if let Some(Ok(lost)) = lost {
            state.dropped = state.dropped.max(u64::try_from(lost).unwrap_or(0));
        }
    }

    /// Ends the stream: at once for a listener; for an event queue, after its thread has taken what the queue still
    /// holds.
    fn finish(&self) {
        let Some(source) = &self.source else {
            wake(self.end_with(None));
            return;
        };
        {
            let mut state = self.lock();
            if state.closed || state.finishing {
                return;
            }
            state.finishing = true;
        }
        self.requests.notify_one();
        // Ends a poll in progress. It fails only when the queue is closed or the SDK is shut down, which end the poll.
        let _ = source.wake_up();
    }

    /// Whether the subscription is attached to a feed of `endpoint` (to any feed when it is `None`), and whether the
    /// endpoints of all its feeds are closed then.
    fn feeds_of(&self, endpoint: Option<&EndpointState>) -> (bool, bool) {
        let Some(feeds) = self.feeds.upgrade() else {
            return (false, false);
        };
        let feeds = crate::lock(&feeds);
        let attached =
            !feeds.is_empty() && endpoint.is_none_or(|endpoint| feeds.iter().any(|feed| feed.is_of(endpoint)));
        (attached, attached && feeds.iter().all(DXFeed::is_closed))
    }

    /// Ends the stream if the endpoints of all the feeds of the subscription are closed and, when `endpoint` is given,
    /// it is one of them; otherwise a stream of an event queue that drained it for `endpoint` alone takes its events
    /// on demand again.
    fn check_feeds(&self, endpoint: Option<&EndpointState>) {
        let drained = {
            let mut state = self.lock();
            if state.closed {
                return;
            }
            let draining = state.draining.len();
            if let Some(endpoint) = endpoint {
                let address = address_of(endpoint);
                state.draining.retain(|&closing| closing != address);
            }
            draining > 0 && state.draining.is_empty()
        };
        match self.feeds_of(endpoint) {
            (_, true) => self.finish(),
            (true, false) => {
                if drained && let Some(source) = &self.source {
                    // Ends the poll that drains, so that the thread polls on demand again.
                    let _ = source.wake_up();
                }
            }
            (false, false) => {}
        }
    }

    /// Makes the thread of a stream of an event queue poll it without demand, if the subscription is attached to a
    /// feed of `endpoint`, which is about to wait for the events that the queue holds.
    fn drain_for(&self, endpoint: &EndpointState) {
        if self.source.is_none() || !self.feeds_of(Some(endpoint)).0 {
            return;
        }
        {
            let mut state = self.lock();
            if state.closed {
                return;
            }
            state.draining.push(address_of(endpoint));
        }
        self.requests.notify_one();
    }

    /// The thread of a stream of an event queue. It polls the queue while a task waits for events, while an endpoint
    /// of the subscription waits for its termination, and before the stream ends; it exits once the stream has ended.
    fn run(self: Arc<Self>) {
        let ran = panic::catch_unwind(AssertUnwindSafe(|| self.poll_source()));
        if ran.is_err()
            && let Some(source) = &self.source
        {
            // The queue that no thread polls any more would hold back the termination of its endpoints.
            let _ = source.close();
            let waker = self.end_with(Some(source.get_dropped_events()));
            error_manager::report_panic(
                "DXFeedSubscription",
                "the waker of an event stream",
                panic::catch_unwind(AssertUnwindSafe(|| wake(waker))),
            );
        }
        error_manager::report_panic("DXFeedSubscription", "the thread of an event stream", ran);
    }

    fn poll_source(&self) {
        let Some(source) = &self.source else {
            return;
        };
        loop {
            let (finishing, timeout) = {
                let mut state = self.lock();
                loop {
                    if state.closed {
                        return;
                    }
                    if state.finishing {
                        break (true, Some(Duration::ZERO));
                    }
                    if state.demand {
                        break (false, None);
                    }
                    // A wake-up goes to a poll that returns events, so the drain must not rely on one to end: it
                    // polls with a short timeout and looks at the requests again.
                    if !state.draining.is_empty() {
                        break (false, Some(DRAIN_POLL));
                    }
                    state = self.requests.wait(state).unwrap_or_else(PoisonError::into_inner);
                }
            };
            match source.poll_items(timeout, BATCH) {
                Ok(PollResult::Events((batch, skipped))) => {
                    let waker = self.push(batch, skipped, Some(source.get_dropped_events()));
                    // A panic of the waker goes to the error manager, as one in a listener does, and the stream goes
                    // on.
                    error_manager::report_panic(
                        "DXFeedSubscription",
                        "the waker of an event stream",
                        panic::catch_unwind(AssertUnwindSafe(|| wake(waker))),
                    );
                }
                // The queue holds nothing more: the stream ends.
                Ok(PollResult::NoEvents) if finishing => {
                    let _ = source.close();
                    return wake(self.end_with(Some(source.get_dropped_events())));
                }
                // A wake-up, or the timeout of a drain: look at the requests again.
                Ok(PollResult::NoEvents) => {}
                Ok(PollResult::Closed) | Err(Error::ShutDown) => {
                    return wake(self.end_with(Some(source.get_dropped_events())));
                }
                Err(error) => {
                    ErrorManager::report_error("DXFeedSubscription", &error);
                    let _ = source.close();
                    return wake(self.end_with(Some(source.get_dropped_events())));
                }
            }
        }
    }
}

/// The address of `endpoint`, which tells it apart from the other endpoints while it is alive.
fn address_of(endpoint: &EndpointState) -> usize {
    std::ptr::from_ref(endpoint) as usize
}

/// Wakes the task of `waker`, if any.
fn wake(waker: Option<Waker>) {
    if let Some(waker) = waker {
        waker.wake();
    }
}

impl<E: SubscriptionItem> Terminate for Queue<E> {
    fn terminate(&self) {
        self.finish();
    }

    fn endpoint_closed(&self, endpoint: &EndpointState) {
        self.check_feeds(Some(endpoint));
    }

    fn endpoint_closing(&self, endpoint: &EndpointState) {
        self.drain_for(endpoint);
    }
}

/// The change listener that ends the stream when the subscription is closed.
struct Closer<E: SubscriptionItem>(Arc<Queue<E>>);

impl<E: SubscriptionItem> ObservableSubscriptionChangeListener for Closer<E> {
    fn symbols_added(&mut self, _symbols: &[Symbol]) {}

    fn subscription_closed(&mut self) {
        self.0.finish();
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
