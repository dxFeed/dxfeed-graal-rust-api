//! Event queues: the events of subscriptions pulled on a thread of the application, without listeners.
//!
//! A queue takes the events of the subscriptions attached to it; a poll waits for them and returns them on the calling
//! thread. An [`EventQueue`] serves one subscription and polls its events as `E`; a [`SharedEventQueue`] serves many,
//! returns each event with the tag of its subscription, and lends its [`Executor`] to the models, the instrument
//! profile collector and the promise handlers, whose listeners then run inside the polls, on the polling thread.
//!
//! The SDK keeps no copy of the events between the polls: it takes them from the buffers of the subscriptions when a
//! poll runs. Until then lasting events, such as quotes, are conflated, and the other events wait in a buffer of the
//! subscription (1000000 events by default, the system property `com.devexperts.qd.impl.matrix.Agent.MaxBufferSize`);
//! beyond it the oldest ones are dropped and counted (`get_dropped_events`). An endpoint of
//! [`Role::StreamFeed`](crate::Role::StreamFeed) drops nothing: it stops receiving data until the queue is polled.
//!
//! Rules of the SDK:
//! - Attach a subscription before it gets symbols while it is attached to a feed, and before it gets an executor: a
//!   subscription with an executor, or one already attached to a queue, is refused with `IllegalStateException`. On a
//!   subscription that has symbols, add the event listeners before the queue: one added after it is refused.
//! - Poll a queue from one thread at a time: the polls are serialized, and a snapshot of indexed events is not split
//!   then.
//! - A poll inside a listener or a callback of the crate returns [`Error::InCallback`].
//! - Closing an endpoint does not close its queues and does not end a poll: poll with a finite timeout, or close the
//!   queue. Close the queue before
//!   [`DXEndpoint::close_and_await_termination`](crate::DXEndpoint::close_and_await_termination) of the endpoint of an
//!   attached subscription, which waits while the queue holds events that no poll returned; never call it on the
//!   polling thread.
//!
//! With the cargo feature `experimental`, a queue also has a ready signal (`set_ready_signal`): an OS object that the
//! SDK makes ready when the queue has work, so that an event loop polls with the timeout zero on its own thread.
//!
//! ```
//! use std::time::Duration;
//!
//! use dxfeed_graal::event::Quote;
//! use dxfeed_graal::event_queue::{EventQueue, PollResult};
//! use dxfeed_graal::{DXEndpoint, Role};
//!
//! let endpoint = DXEndpoint::create_with_role(Role::LocalHub)?;
//! let subscription = endpoint.get_feed().create_subscription::<Quote>()?;
//! let queue = EventQueue::new(&subscription)?;
//! subscription.add_symbols(["AAPL"])?;
//! endpoint.get_publisher().publish_events(&[Quote::new("AAPL").into()])?;
//! match queue.poll(Some(Duration::from_secs(5)), 0)? {
//!     PollResult::Events(quotes) => println!("{quotes:?}"),
//!     PollResult::NoEvents => println!("no quotes yet"),
//!     PollResult::Closed => println!("the queue is closed"),
//! }
//! # Ok::<(), dxfeed_graal::Error>(())
//! ```

use std::marker::PhantomData;
use std::ptr;
use std::time::Duration;

use dxfeed_graal_sys as sys;

use crate::error::{Error, Result};
use crate::event::sdk_event::{take_event_list, take_tagged_event_list};
use crate::event::{Event, SubscriptionItem};
use crate::executor::Executor;
use crate::handle::Handle;
use crate::isolate::{self, call, check, with_thread};
use crate::subscription::DXFeedSubscription;

/// The result of a poll.
#[derive(Debug, Clone, PartialEq)]
pub enum PollResult<T> {
    /// The events that arrived, at least one.
    Events(T),
    /// No events arrived before the timeout or the wake-up; the listeners of the models and the collector on the
    /// queue may have run.
    NoEvents,
    /// The queue is closed: every later poll returns this at once.
    Closed,
}

/// A queue of the events of one subscription (`EventQueue.create(DXFeedSubscription)`): a poll returns them as `E`, in
/// their order.
///
/// The queue closes when the subscription closes, when [`close`](Self::close) is called and when it is dropped. The
/// subscription keeps the executor of the queue for its life, also after the queue is closed. The queue can be shared
/// by threads, but poll it from one thread at a time.
pub struct EventQueue<E: SubscriptionItem = Event> {
    handle: Handle<sys::dxfg_v4_event_queue_t>,
    _item: PhantomData<fn() -> E>,
}

impl<E: SubscriptionItem> EventQueue<E> {
    /// Creates a queue of the events of `subscription`.
    ///
    /// Create it before the symbols are added to a subscription attached to a feed. A subscription with an executor or
    /// a queue, or one attached to a feed that has symbols, is [`Error::Java`] of `java.lang.IllegalStateException`;
    /// on a subscription that has symbols, add its event listeners first.
    #[doc(alias = "create")]
    pub fn new(subscription: &DXFeedSubscription<E>) -> Result<Self> {
        let s = subscription.as_ptr();
        // SAFETY: the subscription is live; the out-pointer is valid for writes.
        let handle = Handle::create(|t, out| unsafe { sys::dxfg_v4_EventQueue_new(t, s, out) })?;
        Ok(EventQueue {
            handle,
            _item: PhantomData,
        })
    }

    /// Makes the poll in progress, or the next one, return [`PollResult::NoEvents`] at once unless it has events;
    /// several wake-ups before a poll wake up one poll.
    #[doc(alias = "wakeUp")]
    pub fn wake_up(&self) -> Result<()> {
        wake_up(&self.handle)
    }

    /// Closes the queue: the poll in progress returns the events it has or [`PollResult::Closed`], and the events that
    /// no poll returned are dropped and counted. It does not wait for the poll in progress; closing a closed queue
    /// does nothing.
    pub fn close(&self) -> Result<()> {
        close(&self.handle)
    }

    /// The number of the events of the subscription that were lost, which never decreases: if it grew since the
    /// previous poll, events were lost before those that the last poll returned.
    #[doc(alias = "getDroppedEvents")]
    pub fn get_dropped_events(&self) -> Result<i64> {
        dropped_events(&self.handle)
    }

    /// Sets, replaces or removes the ready signal of the queue (`dxfg_v4_EventQueue_setReadySignal`); see
    /// [`ReadySignal`]. Needs the cargo feature `experimental`.
    ///
    /// # Safety
    ///
    /// The object of the signal must stay open, and be the same object, until another signal replaces it,
    /// [`ReadySignal::None`] removes it, [`close`](Self::close) returns on any thread or a poll returns
    /// [`PollResult::Closed`]: the SDK writes to it until then. Dropping the queue or closing its subscription does not
    /// count; to stop an event loop, close the queue, and close the object after a poll returns `Closed`.
    #[cfg(feature = "experimental")]
    #[doc(alias = "setReadySignal")]
    pub unsafe fn set_ready_signal(&self, signal: ReadySignal) -> Result<()> {
        set_ready_signal(&self.handle, signal)
    }
}

impl<E: SubscriptionItem + TryFrom<Event>> EventQueue<E> {
    /// Waits for the events of the subscription and returns them; the event listeners of the subscription run in the
    /// poll, on this thread.
    ///
    /// `timeout` is how long to wait: zero returns at once, `None` waits until events arrive, the queue is woken up or
    /// closed. After the first events the poll only completes the retrieval in progress. `limit` stops the retrieval
    /// after that many events, `0` for no limit; a batch may exceed it. A failure of the SDK drops the events that the
    /// poll took, counted in [`get_dropped_events`](Self::get_dropped_events), and the next poll goes on with the next
    /// ones; an event that the crate cannot copy fails the poll and drops its batch without counting it.
    ///
    /// Inside a listener or a callback of the crate it returns [`Error::InCallback`].
    pub fn poll(&self, timeout: Option<Duration>, limit: usize) -> Result<PollResult<Vec<E>>> {
        let q = self.handle.as_ptr();
        let (timeout, limit) = (timeout_millis(timeout), limit_of(limit));
        let mut list = ptr::null_mut();
        // SAFETY: the queue is live; the out-pointer is valid for writes.
        let result = poll(|t| unsafe { sys::dxfg_v4_EventQueue_poll(t, q, timeout, limit, &mut list) })?;
        Ok(match result {
            // SAFETY: on events the SDK returned a list that is ours.
            Polled::Events => events(unsafe { take_event_list(list) }?),
            Polled::NoEvents => PollResult::NoEvents,
            Polled::Closed => PollResult::Closed,
        })
    }
}

impl<E: SubscriptionItem> std::fmt::Debug for EventQueue<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventQueue").finish_non_exhaustive()
    }
}

/// A queue of the events of many subscriptions, each attached with a tag (`EventQueue.create()`): a poll returns
/// each event with the tag of its subscription. Its [`Executor`] runs the work of models, of the instrument profile
/// collector and of promise handlers inside the polls.
///
/// A subscription leaves the queue only by being closed; the queue stays open then. The queue closes when
/// [`close`](Self::close) is called, or when it and every executor it lent are dropped. The queue can be shared by
/// threads, but poll it from one thread at a time.
pub struct SharedEventQueue {
    handle: Handle<sys::dxfg_v4_event_queue_t>,
}

impl SharedEventQueue {
    /// Creates a queue with no subscription.
    #[doc(alias = "create")]
    pub fn new() -> Result<Self> {
        // SAFETY: the out-pointer is valid for writes.
        let handle = Handle::create(|t, out| unsafe { sys::dxfg_v4_EventQueue_create(t, out) })?;
        Ok(SharedEventQueue { handle })
    }

    /// Attaches `subscription`, whose events then come with `tag`; tags need not be unique. A subscription of any
    /// endpoint may be attached.
    ///
    /// Attach it before the symbols are added to a subscription attached to a feed. A subscription with an executor or
    /// a queue, or one attached to a feed that has symbols, is [`Error::Java`] of `java.lang.IllegalStateException`. A
    /// closed subscription, or one attached to a closed queue, is attached as left: its events are dropped and counted.
    /// On a subscription that has symbols, add its event listeners first.
    pub fn attach<E: SubscriptionItem>(&self, subscription: &DXFeedSubscription<E>, tag: i64) -> Result<()> {
        let (q, s) = (self.handle.as_ptr(), subscription.as_ptr());
        // SAFETY: the queue and the subscription are live.
        call(|t| unsafe { sys::dxfg_v4_EventQueue_attach(t, q, s, tag) })
    }

    /// The executor of the queue: the work posted to it runs inside the polls, on the polling thread. Give it to the
    /// models, the collector, the endpoint builder or [`Promise::when_done_async`](crate::Promise::when_done_async).
    ///
    /// The executor keeps the queue open as the queue itself does; the executor of a closed queue runs the work at
    /// once, on the posting thread, without calling the listeners of the application. A subscription that has it as
    /// its executor gets its events by its listeners, not by the polls. The calls of a model that notify on the
    /// calling thread (`set_symbol`, `clear`) call its listener there, not in a poll.
    ///
    /// For a subscription that has it as its executor, also inherited from an endpoint built with it,
    /// [`DXFeed::detach_subscription_and_clear`](crate::DXFeed::detach_subscription_and_clear) and
    /// [`DXEndpoint::close_and_await_termination`](crate::DXEndpoint::close_and_await_termination) wait until a poll
    /// runs the pending work or the queue is closed: never call them on the polling thread.
    #[doc(alias = "getExecutor")]
    pub fn get_executor(&self) -> Result<Executor> {
        let q = self.handle.as_ptr();
        // SAFETY: the queue is live; the out-pointer is valid for writes.
        let handle = Handle::create(|t, out| unsafe { sys::dxfg_v4_EventQueue_getExecutor(t, q, out) })?;
        Ok(Executor::from_handle(handle))
    }

    /// Waits for the events of the attached subscriptions and the work of the executor, and returns the events with
    /// their tags, as [`EventQueue::poll`] does; the listeners of the subscriptions and of the models on the queue run
    /// in the poll, on this thread. [`PollResult::NoEvents`] also comes when only the work of the executor ran.
    ///
    /// Inside a listener or a callback of the crate it returns [`Error::InCallback`].
    #[doc(alias = "pollTagged")]
    pub fn poll(&self, timeout: Option<Duration>, limit: usize) -> Result<PollResult<Vec<(i64, Event)>>> {
        let q = self.handle.as_ptr();
        let (timeout, limit) = (timeout_millis(timeout), limit_of(limit));
        let mut list = ptr::null_mut();
        // SAFETY: the queue is live; the out-pointer is valid for writes.
        let result = poll(|t| unsafe { sys::dxfg_v4_EventQueue_pollTagged(t, q, timeout, limit, &mut list) })?;
        Ok(match result {
            // SAFETY: on events the SDK returned a list that is ours.
            Polled::Events => events(unsafe { take_tagged_event_list(list) }?),
            Polled::NoEvents => PollResult::NoEvents,
            Polled::Closed => PollResult::Closed,
        })
    }

    /// Makes the poll in progress, or the next one, return [`PollResult::NoEvents`] at once unless it has events;
    /// several wake-ups before a poll wake up one poll.
    #[doc(alias = "wakeUp")]
    pub fn wake_up(&self) -> Result<()> {
        wake_up(&self.handle)
    }

    /// Closes the queue, as [`EventQueue::close`] does; the pending work of the executor runs on this thread without
    /// calling the listeners of the application, and its later work runs at once on the posting thread.
    pub fn close(&self) -> Result<()> {
        close(&self.handle)
    }

    /// The number of the events of the attached subscriptions that were lost, which never decreases; it is one for
    /// the whole queue, see [`DXFeedSubscription::get_total_dropped_events`] for each subscription.
    #[doc(alias = "getDroppedEvents")]
    pub fn get_dropped_events(&self) -> Result<i64> {
        dropped_events(&self.handle)
    }

    /// Sets, replaces or removes the ready signal of the queue, as [`EventQueue::set_ready_signal`] does; the work of
    /// the executor also signals. Needs the cargo feature `experimental`.
    ///
    /// # Safety
    ///
    /// As for [`EventQueue::set_ready_signal`]: the object must stay open until another signal replaces it,
    /// [`ReadySignal::None`] removes it, [`close`](Self::close) returns or a poll returns [`PollResult::Closed`]. A
    /// live executor of the queue keeps it open, so dropping the queue alone does not stop the writes.
    #[cfg(feature = "experimental")]
    #[doc(alias = "setReadySignal")]
    pub unsafe fn set_ready_signal(&self, signal: ReadySignal) -> Result<()> {
        set_ready_signal(&self.handle, signal)
    }
}

/// The ready signal of an event queue: the OS object that the SDK makes ready whenever the queue has work (events, the
/// work of its executor, a wake-up, the close), so that an event loop, such as `epoll` or
/// `RegisterWaitForSingleObject`, polls the queue with the timeout zero on its own thread. Needs the cargo feature
/// `experimental`: the SDK function is experimental and may change in a minor release.
///
/// The application creates the object, owns it and closes it. A signal disarms the queue and each poll arms it again,
/// signalling at once if work remains: reset the object before each poll. A signal without work is possible; its poll
/// returns [`PollResult::NoEvents`]. A queue has one signal, and the SDK does not write to an old one after
/// `set_ready_signal` returns; the close signals once more and removes the signal. A signal set on a closed queue is
/// written once and not kept. An object that the SDK refuses, such as a descriptor that is negative, closed, blocking
/// or read-only, or a handle that is null or `INVALID_HANDLE_VALUE`, is [`Error::Java`] of
/// `java.lang.IllegalArgumentException`, and the signal stays as it was. On Windows the value holds a raw handle, so
/// it is neither `Send` nor `Sync`.
#[cfg(feature = "experimental")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReadySignal {
    /// No signal: removes the signal of the queue.
    None,
    /// A non-blocking descriptor open for writing, such as an `eventfd` made with `EFD_NONBLOCK` or the write end of
    /// a non-blocking pipe: the SDK writes the 8 bytes of the value 1 to it.
    #[cfg(unix)]
    Fd(std::os::fd::RawFd),
    /// A Windows event: the SDK calls `SetEvent` on it.
    #[cfg(windows)]
    WindowsEvent(std::os::windows::io::RawHandle),
}

impl std::fmt::Debug for SharedEventQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedEventQueue").finish_non_exhaustive()
    }
}

/// The outcome of an SDK poll that did not fail.
enum Polled {
    Events,
    NoEvents,
    Closed,
}

/// Runs an SDK poll, which returns `0` with events, `DXFG_EVENT_QUEUE_NO_EVENTS`, `DXFG_EVENT_QUEUE_CLOSED`, or `-1`
/// with a thread exception.
fn poll(f: impl FnOnce(isolate::Thread) -> i32) -> Result<Polled> {
    if isolate::in_callback() {
        return Err(Error::InCallback);
    }
    const NO_EVENTS: i32 = sys::DXFG_EVENT_QUEUE_NO_EVENTS as i32;
    const CLOSED: i32 = sys::DXFG_EVENT_QUEUE_CLOSED as i32;
    with_thread(|t| match f(t) {
        NO_EVENTS => Ok(Polled::NoEvents),
        CLOSED => Ok(Polled::Closed),
        code => check(t, code).map(|()| Polled::Events),
    })?
}

/// The result of a poll that has `events`: the SDK reports no events otherwise, so an empty list is no events.
fn events<T>(events: Vec<T>) -> PollResult<Vec<T>> {
    if events.is_empty() {
        PollResult::NoEvents
    } else {
        PollResult::Events(events)
    }
}

/// The timeout of an SDK poll: `-1` waits for ever, a part of a millisecond counts as one.
fn timeout_millis(timeout: Option<Duration>) -> i64 {
    timeout.map_or(-1, |t| {
        i64::try_from(t.as_nanos().div_ceil(1_000_000)).unwrap_or(i64::MAX)
    })
}

/// The limit of an SDK poll; one beyond `i32` is no stricter than `i32::MAX`.
fn limit_of(limit: usize) -> i32 {
    i32::try_from(limit).unwrap_or(i32::MAX)
}

fn wake_up(handle: &Handle<sys::dxfg_v4_event_queue_t>) -> Result<()> {
    let q = handle.as_ptr();
    // SAFETY: the queue is live.
    call(|t| unsafe { sys::dxfg_v4_EventQueue_wakeUp(t, q) })
}

fn close(handle: &Handle<sys::dxfg_v4_event_queue_t>) -> Result<()> {
    let q = handle.as_ptr();
    // SAFETY: the queue is live.
    call(|t| unsafe { sys::dxfg_v4_EventQueue_close(t, q) })
}

#[cfg(feature = "experimental")]
fn set_ready_signal(handle: &Handle<sys::dxfg_v4_event_queue_t>, signal: ReadySignal) -> Result<()> {
    let (kind, object) = match signal {
        ReadySignal::None => (sys::DXFG_V4_READY_SIGNAL_NONE, 0),
        #[cfg(unix)]
        ReadySignal::Fd(fd) => (sys::DXFG_V4_READY_SIGNAL_FD, i64::from(fd)),
        #[cfg(windows)]
        ReadySignal::WindowsEvent(event) => (sys::DXFG_V4_READY_SIGNAL_WINDOWS_EVENT, event as isize as i64),
    };
    let q = handle.as_ptr();
    // SAFETY: the queue is live; the caller keeps the object open while the SDK writes to it.
    call(|t| unsafe { sys::dxfg_v4_EventQueue_setReadySignal(t, q, kind, object) })
}

fn dropped_events(handle: &Handle<sys::dxfg_v4_event_queue_t>) -> Result<i64> {
    let (q, mut dropped) = (handle.as_ptr(), 0);
    // SAFETY: the queue is live; the out-pointer is valid for writes.
    call(|t| unsafe { sys::dxfg_v4_EventQueue_getDroppedEvents(t, q, &mut dropped) })?;
    Ok(dropped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_timeout_rounds_up_and_saturates() {
        assert_eq!(timeout_millis(None), -1);
        assert_eq!(timeout_millis(Some(Duration::ZERO)), 0);
        assert_eq!(timeout_millis(Some(Duration::from_micros(1))), 1);
        assert_eq!(timeout_millis(Some(Duration::from_millis(1500))), 1500);
        assert_eq!(timeout_millis(Some(Duration::MAX)), i64::MAX);
        assert_eq!(limit_of(0), 0);
        assert_eq!(limit_of(usize::MAX), i32::MAX);
    }
}
