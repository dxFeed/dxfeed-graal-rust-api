//! The futures of [`Promise`] and [`AnyOf`]: both implement [`IntoFuture`].

use std::fmt;
use std::future::{Future, IntoFuture};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker, ready};

use super::{AnyOf, Promise, PromiseResult};
use crate::error::{Error, Result};
use crate::terminate::{self, Terminate};

/// The completion of a future, shared with what completes it: the handler of its promise, its `cancel`, or
/// [`system::shutdown`](crate::system::shutdown). The first completion wins.
pub(crate) struct Signal {
    state: Mutex<SignalState>,
}

#[derive(Default)]
struct SignalState {
    done: bool,
    /// The error the future returns instead of the result of its promise.
    error: Option<Error>,
    /// The waker of the task that awaits the future.
    waker: Option<Waker>,
}

impl Signal {
    /// A signal registered for shutdown, or already completed with [`Error::ShutDown`] after it.
    fn registered() -> Arc<Self> {
        let signal = Arc::new(Signal {
            state: Mutex::new(SignalState::default()),
        });
        terminate::register(&signal);
        signal
    }

    /// Completes the future unless it is done: with `error`, or with the result of its promise for `None`. Wakes the
    /// task after the lock is released.
    pub(crate) fn finish(&self, error: Option<Error>) {
        let waker = {
            let mut state = crate::lock(&self.state);
            if state.done {
                return;
            }
            state.done = true;
            state.error = error;
            state.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    fn is_done(&self) -> bool {
        crate::lock(&self.state).done
    }

    /// Ready with the error once done; otherwise keeps the waker of `context` and is pending. The check and the store
    /// happen under one lock, so no completion is missed between them.
    fn poll(&self, context: &mut Context<'_>) -> Poll<Option<Error>> {
        let replaced = {
            let mut state = crate::lock(&self.state);
            if state.done {
                return Poll::Ready(state.error.take());
            }
            match &state.waker {
                Some(waker) if waker.will_wake(context.waker()) => None,
                _ => state.waker.replace(context.waker().clone()),
            }
        };
        // A waker is dropped outside the lock: its drop may run code of the runtime.
        drop(replaced);
        Poll::Pending
    }

    /// Drops the waker of a future that is dropped: the handler of a promise that never completes keeps the signal.
    fn forget_waker(&self) {
        let waker = crate::lock(&self.state).waker.take();
        drop(waker);
    }
}

impl Terminate for Signal {
    fn terminate(&self) {
        self.finish(Some(Error::ShutDown));
    }
}

/// The future of a [`Promise`]: `promise.await` gives its result as [`Promise::get_result`] does, without blocking a
/// thread.
///
/// It owns the promise, so [`cancel`](Self::cancel), [`is_done`](Self::is_done) and
/// [`is_cancelled`](Self::is_cancelled) stay at hand while it is awaited by reference, `(&mut future).await`, as in
/// `select!`. It works on any executor and needs no cargo feature.
///
/// - Dropping the future does not cancel the promise, as dropping a promise does not.
/// - A [`wait`](Promise::wait) with a timeout cancels the promise; an async timeout is a timer of the runtime that
///   calls [`cancel`](Self::cancel) when it fires (see the example).
/// - The handler of a feed request runs on the executor of its endpoint: on a [`ManualExecutor`](crate::ManualExecutor)
///   the future becomes ready inside [`process_all_pending`](crate::ManualExecutor::process_all_pending).
/// - Closing the endpoint does not complete a pending feed request, so its future stays pending until it is
///   cancelled.
/// - [`system::shutdown`](crate::system::shutdown) makes a pending future ready with [`Error::ShutDown`].
///
/// Polling after the future gave its result panics.
///
/// # Examples
///
/// The last quote of a symbol, or [`Error::Timeout`] if the `timer` of the runtime, such as `tokio::time::sleep`, fires
/// first; with tokio, `tokio::select!` on `&mut quote` does the same.
///
/// ```no_run
/// use std::future::{Future, IntoFuture, poll_fn};
/// use std::pin::{Pin, pin};
/// use std::task::Poll;
///
/// use dxfeed_graal::event::Quote;
/// use dxfeed_graal::{DXFeed, Error, Result};
///
/// async fn last_quote(feed: &DXFeed, timer: impl Future<Output = ()>) -> Result<Quote> {
///     let mut quote = feed.get_last_event_promise::<Quote>("AAPL")?.into_future();
///     let mut timer = pin!(timer);
///     poll_fn(|context| {
///         if let Poll::Ready(result) = Pin::new(&mut quote).poll(context) {
///             return Poll::Ready(result);
///         }
///         if timer.as_mut().poll(context).is_ready() {
///             return Poll::Ready(quote.cancel().and(Err(Error::Timeout)));
///         }
///         Poll::Pending
///     })
///     .await
/// }
/// ```
#[must_use = "futures do nothing unless polled"]
pub struct PromiseFuture<T: PromiseResult> {
    promise: Promise<T>,
    signal: Arc<Signal>,
    /// Whether the future gave its result.
    taken: bool,
}

impl<T: PromiseResult> IntoFuture for Promise<T> {
    type Output = Result<T>;
    type IntoFuture = PromiseFuture<T>;

    /// Turns the promise into its future, which adds one handler to it with [`when_done`](Promise::when_done). An
    /// error of that call is the result of the future.
    fn into_future(self) -> PromiseFuture<T> {
        let signal = Signal::registered();
        if !signal.is_done() {
            let handler = Arc::clone(&signal);
            // The handler of a done promise runs before `when_done` returns, so the signal exists before it.
            if let Err(error) = self.when_done(move |_| handler.finish(None)) {
                signal.finish(Some(error));
            }
        }
        PromiseFuture {
            promise: self,
            signal,
            taken: false,
        }
    }
}

impl<T: PromiseResult> PromiseFuture<T> {
    /// Cancels the promise unless it has completed (`Promise.cancel`) and makes the future ready at once, without
    /// waiting for the executor of the endpoint: with [`Error::Java`] of a `java.util.concurrent.CancellationException`,
    /// or with the result if the promise had completed.
    pub fn cancel(&self) -> Result<()> {
        self.promise.cancel()?;
        self.signal.finish(None);
        Ok(())
    }

    /// Whether the promise has completed: with the result, exceptionally or by cancellation (`Promise.isDone`).
    #[doc(alias = "isDone")]
    pub fn is_done(&self) -> Result<bool> {
        self.promise.is_done()
    }

    /// Whether the promise was cancelled (`Promise.isCancelled`).
    #[doc(alias = "isCancelled")]
    pub fn is_cancelled(&self) -> Result<bool> {
        self.promise.is_cancelled()
    }
}

impl<T: PromiseResult> Future for PromiseFuture<T> {
    type Output = Result<T>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Result<T>> {
        let this = self.get_mut();
        assert!(!this.taken, "PromiseFuture polled after completion");
        let error = ready!(this.signal.poll(context));
        this.taken = true;
        Poll::Ready(match error {
            Some(error) => Err(error),
            None => this.promise.completed_result(),
        })
    }
}

impl<T: PromiseResult> Drop for PromiseFuture<T> {
    fn drop(&mut self) {
        self.signal.forget_waker();
    }
}

impl<T: PromiseResult> fmt::Debug for PromiseFuture<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PromiseFuture")
            .field("ready", &self.signal.is_done())
            .field("taken", &self.taken)
            .finish()
    }
}

/// The future of an [`AnyOf`]: `any_of.await` gives the index and the result of the first promise done, as
/// [`AnyOf::get_result`] does, without blocking a thread.
///
/// It owns the [`AnyOf`] and behaves as a [`PromiseFuture`] does: dropping it cancels nothing,
/// [`cancel`](Self::cancel) makes it ready at once, and [`system::shutdown`](crate::system::shutdown) makes a pending
/// one ready with [`Error::ShutDown`]. Polling after it gave its result panics.
#[must_use = "futures do nothing unless polled"]
pub struct AnyOfFuture<T: PromiseResult> {
    any_of: AnyOf<T>,
    signal: Arc<Signal>,
    /// Whether the future gave its result.
    taken: bool,
}

impl<T: PromiseResult> IntoFuture for AnyOf<T> {
    type Output = Result<(usize, T)>;
    type IntoFuture = AnyOfFuture<T>;

    fn into_future(self) -> AnyOfFuture<T> {
        let signal = Signal::registered();
        self.state.attach(Arc::clone(&signal));
        AnyOfFuture {
            any_of: self,
            signal,
            taken: false,
        }
    }
}

impl<T: PromiseResult> AnyOfFuture<T> {
    /// Cancels all the promises, as [`AnyOf::cancel`] does, which makes the future ready at once.
    pub fn cancel(&self) -> Result<()> {
        self.any_of.cancel()
    }

    /// Whether one of the promises is done (`Promise.isDone`).
    #[doc(alias = "isDone")]
    pub fn is_done(&self) -> bool {
        self.any_of.is_done()
    }
}

impl<T: PromiseResult> Future for AnyOfFuture<T> {
    type Output = Result<(usize, T)>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Result<(usize, T)>> {
        let this = self.get_mut();
        assert!(!this.taken, "AnyOfFuture polled after completion");
        let error = ready!(this.signal.poll(context));
        this.taken = true;
        Poll::Ready(match error {
            Some(error) => Err(error),
            // The signal completes after the index is set.
            None => this.any_of.get_result().and_then(|result| result.ok_or(Error::Timeout)),
        })
    }
}

impl<T: PromiseResult> Drop for AnyOfFuture<T> {
    fn drop(&mut self) {
        self.signal.forget_waker();
    }
}

impl<T: PromiseResult> fmt::Debug for AnyOfFuture<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnyOfFuture")
            .field("any_of", &self.any_of)
            .field("taken", &self.taken)
            .finish()
    }
}
