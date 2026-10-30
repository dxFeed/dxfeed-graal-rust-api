//! The promise of a last event.

use std::marker::PhantomData;
use std::ptr;
use std::time::Duration;

use dxfeed_graal_sys as sys;

use crate::error::{Error, JavaException, Result};
use crate::event::LastingEvent;
use crate::event::sdk_event::take_event;
use crate::feed::DXFeed;
use crate::handle::Handle;
use crate::isolate::{self, Thread, call};

/// What `dxfg_v4_Promise_awaitWithoutException` returns when the wait timed out (`DXFG_PROMISE_AWAIT_TIMED_OUT`).
const AWAIT_TIMED_OUT: i32 = 1;

/// The result of [`DXFeed::get_last_event_promise`] that becomes known later (`Promise`).
///
/// [`wait`](Self::wait) blocks for it; [`result`](Self::result), [`is_done`](Self::is_done),
/// [`is_cancelled`](Self::is_cancelled) and [`cancel`](Self::cancel) do not block. Dropping a promise does not cancel
/// the request, as in Java. The promise keeps the endpoint of its feed open while it lives.
pub struct Promise<E: LastingEvent> {
    handle: Handle<sys::dxfg_v4_promise_event_t>,
    _feed: DXFeed,
    _event: PhantomData<fn() -> E>,
}

impl<E: LastingEvent> Promise<E> {
    pub(crate) fn new(handle: Handle<sys::dxfg_v4_promise_event_t>, feed: DXFeed) -> Self {
        Promise {
            handle,
            _feed: feed,
            _event: PhantomData,
        }
    }

    /// Waits at most `timeout` for the event (`Promise.awaitWithoutException`).
    ///
    /// Returns [`Error::Timeout`] when the timeout expires: a positive timeout then cancels the promise, as in Java,
    /// while a zero timeout only checks it and leaves it pending. Returns [`Error::Java`] when the promise has
    /// completed exceptionally or was cancelled. The timeout is rounded up to whole milliseconds; one over 24 days
    /// waits 24 days.
    ///
    /// A positive timeout inside an event listener returns [`Error::InCallback`]: the listener thread delivers the
    /// events the promise may be waiting for.
    #[doc(alias = "await")]
    #[doc(alias = "awaitWithoutException")]
    pub fn wait(&self, timeout: Duration) -> Result<E> {
        if timeout.is_zero() {
            return self.result()?.ok_or(Error::Timeout);
        }
        if isolate::in_callback() {
            return Err(Error::InCallback);
        }
        let millis = i32::try_from(timeout.as_nanos().div_ceil(1_000_000)).unwrap_or(i32::MAX);
        let promise = self.as_promise();
        let mut timed_out = false;
        call(|t| {
            // SAFETY: the promise is live.
            let code = unsafe { sys::dxfg_v4_Promise_awaitWithoutException(t, promise, millis) };
            timed_out = code == AWAIT_TIMED_OUT;
            if timed_out { 0 } else { code }
        })?;
        if timed_out {
            return Err(Error::Timeout);
        }
        self.completed_result()
    }

    /// The event if the promise has completed with it, `None` while it is pending (`Promise.getResult`).
    ///
    /// Returns [`Error::Java`] when the promise has completed exceptionally or was cancelled.
    #[doc(alias = "getResult")]
    pub fn result(&self) -> Result<Option<E>> {
        if !self.is_done()? {
            return Ok(None);
        }
        self.completed_result().map(Some)
    }

    /// The event of a completed promise, or its exception.
    fn completed_result(&self) -> Result<E> {
        let promise = self.handle.as_ptr();
        // SAFETY: the promise is live and the out-pointer is valid for writes.
        if let Some(event) = take_event(|t, out| unsafe { sys::dxfg_v4_Promise_EventType_getResult(t, promise, out) })?
        {
            return Ok(event);
        }
        Err(Error::Java(self.exception()?.unwrap_or_else(JavaException::lost)))
    }

    /// Whether the promise has completed: with the event, exceptionally or by cancellation (`Promise.isDone`).
    #[doc(alias = "isDone")]
    pub fn is_done(&self) -> Result<bool> {
        self.flag(sys::dxfg_v4_Promise_isDone)
    }

    /// Whether the promise was cancelled: by [`cancel`](Self::cancel) or a [`wait`](Self::wait) that timed out
    /// (`Promise.isCancelled`).
    #[doc(alias = "isCancelled")]
    pub fn is_cancelled(&self) -> Result<bool> {
        self.flag(sys::dxfg_v4_Promise_isCancelled)
    }

    /// Cancels the promise unless it has completed; a [`wait`](Self::wait) in another thread then returns
    /// [`Error::Java`] (`Promise.cancel`).
    pub fn cancel(&self) -> Result<()> {
        let promise = self.as_promise();
        // SAFETY: the promise is live.
        call(|t| unsafe { sys::dxfg_v4_Promise_cancel(t, promise) })
    }

    /// The promise as the functions of all promises take it.
    fn as_promise(&self) -> *mut sys::dxfg_v4_promise_t {
        // SAFETY: the promise is live while `self` lives.
        unsafe { &raw mut (*self.handle.as_ptr()).handler }
    }

    /// Reads a flag of the promise with an SDK function.
    fn flag(&self, f: unsafe extern "C" fn(Thread, *mut sys::dxfg_v4_promise_t, *mut i32) -> i32) -> Result<bool> {
        let promise = self.as_promise();
        let mut flag = 0;
        // SAFETY: the promise is live and the out-pointer is valid for writes.
        call(|t| unsafe { f(t, promise, &mut flag) })?;
        Ok(flag != 0)
    }

    /// The exception of the promise: `CancellationException` for a cancelled one, `None` while it is pending or after
    /// it has completed with the event.
    fn exception(&self) -> Result<Option<JavaException>> {
        let promise = self.as_promise();
        let mut exception = ptr::null_mut();
        // SAFETY: the promise is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_Promise_getException(t, promise, &mut exception) })?;
        // SAFETY: the exception is NULL or new, and nothing else uses it.
        Ok(unsafe { isolate::take_returned_exception(exception) })
    }
}
