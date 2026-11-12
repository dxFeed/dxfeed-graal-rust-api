//! Promises: of the result of a feed request (a last event or a list of events), or created by the application.

use std::ffi::c_void;
use std::marker::PhantomData;
use std::panic::{self, AssertUnwindSafe};
use std::ptr;
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::Duration;

use dxfeed_graal_sys as sys;

use crate::error::{Error, JavaException, JavaExceptionClass, Result, c_string};
use crate::event::sdk_event::{take_event, take_events};
use crate::event::{Event, EventKind, IndexedEvent, LastingEvent, SysEvents};
use crate::executor::Executor;
use crate::feed::DXFeed;
use crate::handle::Handle;
use crate::isolate::{self, CallbackScope, Thread, call};

mod future;

use future::Signal;
pub use future::{AnyOfFuture, PromiseFuture};

/// What `dxfg_v4_Promise_awaitWithoutException` returns when the wait timed out (`DXFG_PROMISE_AWAIT_TIMED_OUT`).
const AWAIT_TIMED_OUT: i32 = 1;

/// The result of a feed request that becomes known later (`Promise`): an event of [`DXFeed::get_last_event_promise`],
/// or a `Vec` of events of [`DXFeed::get_time_series_promise`] and [`DXFeed::get_indexed_events_promise`].
/// [`Promise::new`] creates one that the application completes, and [`Promise::failed`] one that has failed.
///
/// [`wait`](Self::wait) and [`await_result`](Self::await_result) block for it; [`get_result`](Self::get_result),
/// [`is_done`](Self::is_done), [`is_cancelled`](Self::is_cancelled) and [`cancel`](Self::cancel) do not block, and
/// [`when_done`](Self::when_done) runs a closure when it is done. [`Promises`] combines promises. Dropping a promise
/// does not cancel the request, as in Java. The promise keeps the endpoint of its feed open while it lives.
///
/// In async code, `promise.await` gives the result without blocking a thread: the promise turns into a
/// [`PromiseFuture`], which keeps [`cancel`](PromiseFuture::cancel) at hand.
pub struct Promise<T: PromiseResult> {
    handle: Handle<sys::dxfg_v4_promise_t>,
    /// The feeds whose endpoints the promise keeps open: that of its request, or those of the promises of
    /// [`Promises::all_of`].
    feeds: Vec<DXFeed>,
    _result: PhantomData<fn() -> T>,
}

/// What a [`Promise`] completes with: a [`LastingEvent`], a `Vec` of an [`IndexedEvent`] type, or `()` for
/// [`Promises::all_of`].
///
/// The trait is sealed: only this crate implements it.
pub trait PromiseResult: sealed::Sealed {}

mod sealed {
    use dxfeed_graal_sys as sys;

    use crate::error::Result;

    pub trait Sealed: Sized {
        /// Copies the result of a completed promise; `None` if it has none.
        fn take(promise: *mut sys::dxfg_v4_promise_t) -> Result<Option<Self>>;
    }
}

impl<E: LastingEvent + EventKind> PromiseResult for E {}

impl<E: LastingEvent + EventKind> sealed::Sealed for E {
    fn take(promise: *mut sys::dxfg_v4_promise_t) -> Result<Option<Self>> {
        let promise = promise.cast::<sys::dxfg_v4_promise_event_t>();
        // SAFETY: the promise of a single event is live and the out-pointer is valid for writes.
        take_event(|t, out| unsafe { sys::dxfg_v4_Promise_EventType_getResult(t, promise, out) })
    }
}

impl PromiseResult for () {}

impl sealed::Sealed for () {
    fn take(_promise: *mut sys::dxfg_v4_promise_t) -> Result<Option<Self>> {
        Ok(Some(()))
    }
}

impl<E: IndexedEvent + EventKind> PromiseResult for Vec<E> {}

impl<E: IndexedEvent + EventKind> sealed::Sealed for Vec<E> {
    fn take(promise: *mut sys::dxfg_v4_promise_t) -> Result<Option<Self>> {
        let promise = promise.cast::<sys::dxfg_v4_promise_events_t>();
        // SAFETY: the promise of a list is live and the out-pointer is valid for writes.
        take_events(|t, out| unsafe { sys::dxfg_v4_Promise_List_EventType_getResult(t, promise, out) })
    }
}

impl<E: LastingEvent + EventKind> Promise<E> {
    /// Completes the promise with `event` unless it is done, which runs its handlers as [`when_done`](Self::when_done)
    /// describes (`complete`).
    pub fn complete(&self, event: &E) -> Result<()> {
        let events = SysEvents::new(&[event.clone().into()])?;
        let event = events.pointers()[0];
        let p = self.as_promise();
        // SAFETY: the promise of an event is live and the event lives in `events` during the call; the SDK only reads
        // the event.
        call(|t| unsafe { sys::dxfg_v4_Promise_EventType_complete(t, p, event) })
    }
}

impl<E: IndexedEvent + EventKind> Promise<Vec<E>> {
    /// Completes the promise with `events` unless it is done, which runs its handlers as
    /// [`when_done`](Self::when_done) describes (`complete`).
    pub fn complete(&self, events: &[E]) -> Result<()> {
        let events: Vec<Event> = events.iter().cloned().map(Into::into).collect();
        let mut events = SysEvents::new(&events)?;
        let mut list = events.list();
        let p = self.as_promise();
        // SAFETY: the promise of a list is live and the list and its events live in `events` during the call; the SDK
        // only reads them.
        call(|t| unsafe { sys::dxfg_v4_Promise_List_EventType_complete(t, p, &mut list) })
    }
}

impl Promise<()> {
    /// Completes the promise unless it is done, which runs its handlers as [`when_done`](Self::when_done) describes
    /// (`complete`).
    pub fn complete(&self) -> Result<()> {
        let p = self.as_promise();
        // SAFETY: the promise is live; a NULL result is the null object.
        call(|t| unsafe { sys::dxfg_v4_Promise_complete(t, p, ptr::null_mut()) })
    }
}

impl<T: PromiseResult> Promise<T> {
    pub(crate) fn from_request(handle: Handle<sys::dxfg_v4_promise_t>, feed: DXFeed) -> Self {
        Promise {
            handle,
            feeds: vec![feed],
            _result: PhantomData,
        }
    }

    /// Creates a promise that is not done (`new Promise()`). The application completes it with `complete`,
    /// [`complete_exceptionally`](Self::complete_exceptionally) or [`cancel`](Self::cancel); the handlers that
    /// [`when_done`](Self::when_done) adds before then run on the thread that does so, before that call returns. The
    /// promise keeps no endpoint open.
    pub fn new() -> Result<Self> {
        // SAFETY: the out-pointer is valid for writes.
        let handle = Handle::create(|t, out| unsafe { sys::dxfg_v4_Promise_new(t, out) })?;
        Ok(Self::without_feed(handle))
    }

    /// Creates a promise that has failed with a new Java exception of `class` and `message` (`Promise.failed`). The
    /// promise keeps no endpoint open.
    ///
    /// A message with a NUL byte returns [`Error::InvalidArgument`].
    pub fn failed(class: JavaExceptionClass, message: &str) -> Result<Self> {
        let (class, message) = (class.c_name(), c_string(message, "exception message")?);
        // SAFETY: both strings are NUL-terminated and outlive the call; the out-pointer is valid for writes.
        let handle = Handle::create(|t, out| unsafe {
            sys::dxfg_v4_Promise_failedWithClassNameAndMessage(t, class.as_ptr(), message.as_ptr(), out)
        })?;
        Ok(Self::without_feed(handle))
    }

    /// Completes the promise exceptionally with a new Java exception of `class` and `message` unless it is done, which
    /// runs its handlers as [`when_done`](Self::when_done) describes (`completeExceptionally`). The exception has no
    /// cause. [`get_exception`](Self::get_exception), [`get_result`](Self::get_result),
    /// [`await_result`](Self::await_result) and a zero-timeout [`wait`](Self::wait) return it as [`Error::Java`]; a
    /// [`wait`](Self::wait) with a positive timeout returns a `com.dxfeed.promise.PromiseException` caused by it, as
    /// Java's `awaitWithoutException` does.
    ///
    /// A message with a NUL byte returns [`Error::InvalidArgument`].
    #[doc(alias = "completeExceptionally")]
    pub fn complete_exceptionally(&self, class: JavaExceptionClass, message: &str) -> Result<()> {
        let (class, message) = (class.c_name(), c_string(message, "exception message")?);
        let p = self.as_promise();
        // SAFETY: the promise is live and both strings are NUL-terminated and outlive the call.
        call(|t| unsafe {
            sys::dxfg_v4_Promise_completeExceptionallyWithClassNameAndMessage(t, p, class.as_ptr(), message.as_ptr())
        })
    }

    /// A promise of `handle` that keeps no endpoint open.
    fn without_feed(handle: Handle<sys::dxfg_v4_promise_t>) -> Self {
        Promise {
            handle,
            feeds: Vec::new(),
            _result: PhantomData,
        }
    }

    /// Another handle of the promise that keeps no endpoint open.
    fn try_clone(&self) -> Result<Self> {
        Ok(Self::without_feed(self.handle.try_clone()?))
    }

    /// Waits at most `timeout` for the result (`Promise.awaitWithoutException`).
    ///
    /// Returns [`Error::Timeout`] when the timeout expires: a positive timeout then cancels the promise, as in Java,
    /// while a zero timeout only checks it and leaves it pending. Returns [`Error::Java`] when the promise has
    /// completed exceptionally or was cancelled. The timeout is rounded up to whole milliseconds; one over 24 days
    /// waits 24 days.
    ///
    /// A promise shared between threads is cancelled for all of them when the positive timeout of one wait expires;
    /// threads that must not cancel it check it with a zero timeout.
    ///
    /// A positive timeout inside an event listener or a [`when_done`](Self::when_done) closure returns
    /// [`Error::InCallback`]: the listener thread delivers the events the promise may be waiting for.
    #[doc(alias = "await")]
    #[doc(alias = "awaitWithoutException")]
    pub fn wait(&self, timeout: Duration) -> Result<T> {
        if timeout.is_zero() {
            return self.get_result()?.ok_or(Error::Timeout);
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

    /// Waits for the result without a time limit (`Promise.await`).
    ///
    /// Returns [`Error::Java`] when the promise has completed exceptionally or was cancelled, and
    /// [`Error::InCallback`] inside an event listener or a [`when_done`](Self::when_done) closure. Closing the
    /// endpoint does not complete a pending promise of a feed request, so the wait never ends then: prefer
    /// [`wait`](Self::wait) with a timeout.
    #[doc(alias = "await")]
    pub fn await_result(&self) -> Result<T> {
        if isolate::in_callback() {
            return Err(Error::InCallback);
        }
        let promise = self.as_promise();
        // SAFETY: the promise is live.
        let waited = call(|t| unsafe { sys::dxfg_v4_Promise_await(t, promise) });
        match waited {
            Err(error) if !self.is_done()? => Err(error),
            _ => self.completed_result(),
        }
    }

    /// The result if the promise has completed with it, `None` while it is pending (`Promise.getResult`).
    ///
    /// Returns [`Error::Java`] when the promise has completed exceptionally or was cancelled.
    #[doc(alias = "getResult")]
    pub fn get_result(&self) -> Result<Option<T>> {
        if !self.is_done()? {
            return Ok(None);
        }
        self.completed_result().map(Some)
    }

    /// The result of a completed promise, or its exception.
    fn completed_result(&self) -> Result<T> {
        // A promise of a list gives an empty list without a result, so the result is taken only when there is one.
        if self.flag(sys::dxfg_v4_Promise_hasResult)?
            && let Some(result) = T::take(self.handle.as_ptr())?
        {
            return Ok(result);
        }
        Err(Error::Java(self.get_exception()?.unwrap_or_else(JavaException::lost)))
    }

    /// Whether the promise has completed with the result (`Promise.hasResult`).
    #[doc(alias = "hasResult")]
    pub fn has_result(&self) -> Result<bool> {
        self.flag(sys::dxfg_v4_Promise_hasResult)
    }

    /// Whether the promise has completed exceptionally or was cancelled (`Promise.hasException`).
    #[doc(alias = "hasException")]
    pub fn has_exception(&self) -> Result<bool> {
        self.flag(sys::dxfg_v4_Promise_hasException)
    }

    /// Whether the promise has completed: with the result, exceptionally or by cancellation (`Promise.isDone`).
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
        self.handle.as_ptr()
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
    /// it has completed with the result (`Promise.getException`).
    #[doc(alias = "getException")]
    pub fn get_exception(&self) -> Result<Option<JavaException>> {
        let promise = self.as_promise();
        let mut exception = ptr::null_mut();
        // SAFETY: the promise is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_Promise_getException(t, promise, &mut exception) })?;
        // SAFETY: the exception is NULL or new, and nothing else uses it.
        Ok(unsafe { isolate::take_returned_exception(exception) })
    }

    /// Runs `handler` once when the promise is done: completed, completed exceptionally or cancelled
    /// (`Promise.whenDone`).
    ///
    /// If the promise is done, the calling thread runs it before this method returns. Otherwise a thread of the
    /// executor of the endpoint runs it for a feed request, also when the promise is cancelled; for a promise of
    /// [`Promises::all_of`] or of [`Promise::new`], the thread that completes or cancels it does, including a
    /// [`wait`](Self::wait) whose timeout cancels it.
    ///
    /// `handler` receives another handle of the promise, which keeps no endpoint open. A panic of `handler` goes to
    /// [`ErrorManager`](crate::ErrorManager); blocking waits inside it return [`Error::InCallback`]. A handler cannot
    /// be removed: one of a promise that never completes, such as a promise whose endpoint was closed, stays with its
    /// closure.
    #[doc(alias = "whenDone")]
    pub fn when_done(&self, handler: impl FnOnce(&Promise<T>) + Send + 'static) -> Result<()> {
        self.add_handler(Box::new(handler), None)
    }

    /// Runs `handler` once on `executor` when the promise is done (`Promise.whenDoneAsync`); otherwise as
    /// [`when_done`](Self::when_done). On a [`ManualExecutor`](crate::ManualExecutor), it runs when the executor
    /// processes its tasks.
    #[doc(alias = "whenDoneAsync")]
    pub fn when_done_async(
        &self,
        handler: impl FnOnce(&Promise<T>) + Send + 'static,
        executor: &Executor,
    ) -> Result<()> {
        self.add_handler(Box::new(handler), Some(executor))
    }

    /// Adds the handler of [`when_done`](Self::when_done) or [`when_done_async`](Self::when_done_async).
    fn add_handler(&self, closure: Box<HandlerClosure<T>>, executor: Option<&Executor>) -> Result<()> {
        let promise = self.try_clone()?;
        let p = promise.as_promise();
        let state: Arc<HandlerState<T>> = Arc::new(Mutex::new(Some(WhenDone { promise, closure })));
        let user_data = Arc::into_raw(Arc::clone(&state)).cast_mut().cast::<c_void>();
        let handler: sys::dxfg_v4_promise_handler_function = Some(when_done_trampoline::<T>);
        let added = match executor {
            // SAFETY: the promise is live until the handler has run: the handler owns it.
            None => call(|t| unsafe { sys::dxfg_v4_Promise_whenDone(t, p, handler, user_data) }),
            Some(executor) => {
                let e = executor.handle.as_ptr();
                // SAFETY: as above; the executor is live during the call.
                call(|t| unsafe { sys::dxfg_v4_Promise_whenDoneAsync(t, p, handler, user_data, e) })
            }
        };
        if added.is_err() {
            // The SDK may have run the handler before it failed, or may still run it: the state is emptied here,
            // and the reference the SDK got stays with it.
            let taken = crate::lock(&state).take();
            drop(taken);
        }
        added
    }
}

/// The closure of [`Promise::when_done`].
type HandlerClosure<T> = dyn FnOnce(&Promise<T>) + Send;

/// The state of a handler of [`Promise::when_done`], which the handler takes when it runs.
type HandlerState<T> = Mutex<Option<WhenDone<T>>>;

/// What a handler of [`Promise::when_done`] runs: the closure and the handle of the promise it receives, which the SDK
/// uses until it calls the handler.
struct WhenDone<T: PromiseResult> {
    promise: Promise<T>,
    closure: Box<HandlerClosure<T>>,
}

/// Runs the closure of a handler of a promise, then frees its state.
unsafe extern "C" fn when_done_trampoline<T: PromiseResult>(
    thread: *mut sys::graal_isolatethread_t,
    _promise: *mut sys::dxfg_promise_t,
    user_data: *mut c_void,
) {
    if isolate::exiting() {
        return;
    }
    // SAFETY: `user_data` is the reference to the state that `add_handler` gave the SDK, which calls the handler at
    // most once.
    let state = unsafe { Arc::from_raw(user_data.cast_const().cast::<HandlerState<T>>()) };
    let Some(WhenDone { promise, closure }) = crate::lock(&state).take() else {
        return;
    };
    let _scope = CallbackScope::enter(thread);
    crate::error_manager::report_panic(
        "Promise",
        "a promise handler",
        panic::catch_unwind(AssertUnwindSafe(|| closure(&promise))),
    );
    // The SDK does not use the promise after the handler returns.
    drop(promise);
}

/// The combinations of promises (`Promises`).
#[derive(Debug)]
pub enum Promises {}

impl Promises {
    /// A promise that completes when all `promises` are done, normally or exceptionally (`Promises.allOf`).
    ///
    /// The promises have one result type, unlike in Java. Its result is `()`: check the results of `promises` one by
    /// one. It is done at once if `promises` is empty.
    /// When it completes for any reason, such as [`cancel`](Promise::cancel), it cancels all `promises`. It keeps
    /// the endpoints of their feeds open while it lives.
    #[doc(alias = "allOf")]
    pub fn all_of<'a, T: PromiseResult + 'a>(
        promises: impl IntoIterator<Item = &'a Promise<T>>,
    ) -> Result<Promise<()>> {
        let promises: Vec<&Promise<T>> = promises.into_iter().collect();
        let size = i32::try_from(promises.len())
            .map_err(|_| Error::InvalidArgument(format!("too many promises: {}", promises.len())))?;
        let mut elements: Vec<*mut sys::dxfg_java_object_handler> =
            promises.iter().map(|promise| promise.as_promise().cast()).collect();
        let mut list = sys::dxfg_v4_promise_list {
            list: sys::dxfg_java_object_handler_list {
                size,
                elements: if elements.is_empty() {
                    ptr::null_mut()
                } else {
                    elements.as_mut_ptr()
                },
            },
        };
        // SAFETY: the promises of the list are live and the SDK only reads it; the out-pointer is valid for writes.
        let handle = Handle::create(|t, out| unsafe { sys::dxfg_v4_Promises_allOf(t, &mut list, out) })?;
        Ok(Promise {
            handle,
            feeds: promises
                .iter()
                .flat_map(|promise| promise.feeds.iter().cloned())
                .collect(),
            _result: PhantomData,
        })
    }

    /// The first of `promises` to be done, normally or exceptionally (`Promises.anyOf`); the others are then
    /// cancelled.
    ///
    /// The result is an [`AnyOf`], which owns `promises` and tells which of them is done. Returns
    /// [`Error::InvalidArgument`] if `promises` is empty.
    #[doc(alias = "anyOf")]
    pub fn any_of<T: PromiseResult + 'static>(promises: Vec<Promise<T>>) -> Result<AnyOf<T>> {
        if promises.is_empty() {
            return Err(Error::InvalidArgument("anyOf needs at least one promise".into()));
        }
        let state = Arc::new(AnyOfState {
            promises,
            first: Mutex::new(First::default()),
            done: Condvar::new(),
        });
        for (index, promise) in state.promises.iter().enumerate() {
            let state: Weak<AnyOfState<T>> = Arc::downgrade(&state);
            promise.when_done(move |_| {
                if let Some(state) = state.upgrade() {
                    state.complete(index);
                }
            })?;
        }
        Ok(AnyOf { state })
    }
}

/// The first of several promises to be done, from [`Promises::any_of`]: it is done when one of them is, and its
/// result is the index and the result of that one.
///
/// It owns the promises. Dropping it does not cancel them, as in Java. In async code, `any_of.await` gives the index
/// and the result through an [`AnyOfFuture`].
pub struct AnyOf<T: PromiseResult> {
    state: Arc<AnyOfState<T>>,
}

/// The state of an [`AnyOf`], which the handlers of its promises share.
struct AnyOfState<T: PromiseResult> {
    promises: Vec<Promise<T>>,
    first: Mutex<First>,
    done: Condvar,
}

/// The first promise done of an [`AnyOf`], and the signal of its future.
#[derive(Default)]
struct First {
    /// The index of the first promise done.
    index: Option<usize>,
    /// The signal of the [`AnyOfFuture`]: the first promise done completes it after cancelling the others; a future
    /// made later completes it at once.
    signal: Option<Arc<Signal>>,
}

impl<T: PromiseResult> AnyOfState<T> {
    /// Notes that the promise at `index` is done; the first one cancels the others, then completes the future.
    fn complete(&self, index: usize) {
        let signal = {
            let mut first = crate::lock(&self.first);
            if first.index.is_some() {
                return;
            }
            first.index = Some(index);
            first.signal.take()
        };
        self.done.notify_all();
        for (other, promise) in self.promises.iter().enumerate() {
            if other != index
                && let Err(error) = promise.cancel()
            {
                crate::ErrorManager::report_error("Promise", &error);
            }
        }
        // A cancellation may run handlers on this thread: the future is woken after them.
        if let Some(signal) = signal {
            signal.finish(None);
        }
    }

    /// Completes `signal` when a promise is done, at once if one is.
    fn attach(&self, signal: Arc<Signal>) {
        let done = {
            let mut first = crate::lock(&self.first);
            if first.index.is_none() {
                first.signal = Some(signal);
                return;
            }
            signal
        };
        done.finish(None);
    }
}

impl<T: PromiseResult> AnyOf<T> {
    /// The promises, in the order given.
    pub fn get_promises(&self) -> &[Promise<T>] {
        &self.state.promises
    }

    /// The index of the first promise done, `None` while all are pending.
    pub fn get_done_index(&self) -> Option<usize> {
        crate::lock(&self.state.first).index
    }

    /// Whether one of the promises is done (`Promise.isDone`).
    #[doc(alias = "isDone")]
    pub fn is_done(&self) -> bool {
        self.get_done_index().is_some()
    }

    /// The index and the result of the first promise done, `None` while all are pending (`Promise.getResult`).
    ///
    /// Returns [`Error::Java`] when that promise has completed exceptionally or was cancelled.
    #[doc(alias = "getResult")]
    pub fn get_result(&self) -> Result<Option<(usize, T)>> {
        match self.get_done_index() {
            Some(index) => Ok(Some((index, self.state.promises[index].completed_result()?))),
            None => Ok(None),
        }
    }

    /// Waits at most `timeout` for the first promise done, as [`Promise::wait`] does: a positive timeout that expires
    /// cancels all the promises and returns [`Error::Timeout`].
    #[doc(alias = "await")]
    pub fn wait(&self, timeout: Duration) -> Result<(usize, T)> {
        if timeout.is_zero() {
            return self.get_result()?.ok_or(Error::Timeout);
        }
        if isolate::in_callback() {
            return Err(Error::InCallback);
        }
        let timeout = timeout.min(Duration::from_millis(i32::MAX as u64));
        let first = crate::lock(&self.state.first);
        let (first, waited) = self
            .state
            .done
            .wait_timeout_while(first, timeout, |first| first.index.is_none())
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        drop(first);
        if waited.timed_out() {
            self.cancel()?;
            return Err(Error::Timeout);
        }
        self.get_result()?.ok_or(Error::Timeout)
    }

    /// Waits without a time limit for the first promise done (`Promise.await`), as [`Promise::await_result`] does.
    pub fn await_result(&self) -> Result<(usize, T)> {
        if isolate::in_callback() {
            return Err(Error::InCallback);
        }
        let first = crate::lock(&self.state.first);
        drop(
            self.state
                .done
                .wait_while(first, |first| first.index.is_none())
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        self.get_result()?.ok_or(Error::Timeout)
    }

    /// Cancels all the promises, which makes this done at once (`Promise.cancel`); returns the first error after trying
    /// them all.
    pub fn cancel(&self) -> Result<()> {
        let mut result = Ok(());
        for promise in &self.state.promises {
            if let Err(error) = promise.cancel()
                && result.is_ok()
            {
                result = Err(error);
            }
        }
        // The handlers of feed requests run later, on the executors of their endpoints.
        if let Some(index) = self
            .state
            .promises
            .iter()
            .position(|promise| promise.is_done().unwrap_or(false))
        {
            self.state.complete(index);
        }
        result
    }
}

impl<T: PromiseResult> std::fmt::Debug for AnyOf<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnyOf")
            .field("promises", &self.state.promises.len())
            .field("done_index", &self.get_done_index())
            .finish()
    }
}
