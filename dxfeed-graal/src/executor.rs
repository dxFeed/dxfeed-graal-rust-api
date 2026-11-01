//! Executors of Java: thread pools, and an executor whose tasks run when the application asks.

use std::ops::Deref;
use std::sync::Arc;

use dxfeed_graal_sys as sys;

use crate::error::{Error, Result, c_string};
use crate::handle::Handle;
use crate::isolate::{Thread, call};

/// An executor of Java that runs the tasks of an endpoint, listener calls included (`java.util.concurrent.Executor`).
///
/// Pass it to [`DXEndpointBuilder::with_executor`](crate::DXEndpointBuilder::with_executor), to the builders of the
/// models, or to a [`SubscriptionController`](crate::SubscriptionController). A thread pool comes from
/// [`new_fixed_thread_pool`](Self::new_fixed_thread_pool) or
/// [`new_scheduled_thread_pool`](Self::new_scheduled_thread_pool); a [`ManualExecutor`] is also an executor. Clones
/// share one executor.
///
/// The threads of a pool are daemon threads that live until the process exits: the SDK cannot shut a pool down.
#[derive(Clone)]
pub struct Executor {
    pub(crate) handle: Arc<Handle<sys::dxfg_v4_executor_t>>,
}

impl Executor {
    /// Creates a pool of `threads` threads named after `name` (`Executors.newFixedThreadPool`).
    ///
    /// Returns [`Error::Java`] for zero threads and [`Error::InvalidArgument`] for more than `i32::MAX` threads or a
    /// name with a NUL byte.
    #[doc(alias = "newFixedThreadPool")]
    pub fn new_fixed_thread_pool(threads: usize, name: &str) -> Result<Executor> {
        Executor::pool(threads, name, sys::dxfg_v4_Executors_newFixedThreadPool)
    }

    /// Creates a pool of `threads` threads named after `name` that can also run tasks with a delay or periodically
    /// (`Executors.newScheduledThreadPool`).
    ///
    /// Returns [`Error::Java`] for zero threads and [`Error::InvalidArgument`] for more than `i32::MAX` threads or a
    /// name with a NUL byte.
    #[doc(alias = "newScheduledThreadPool")]
    pub fn new_scheduled_thread_pool(threads: usize, name: &str) -> Result<Executor> {
        Executor::pool(threads, name, sys::dxfg_v4_Executors_newScheduledThreadPool)
    }

    fn pool(
        threads: usize,
        name: &str,
        new: unsafe extern "C" fn(Thread, i32, *const std::ffi::c_char, *mut *mut sys::dxfg_v4_executor_t) -> i32,
    ) -> Result<Executor> {
        let threads = i32::try_from(threads)
            .map_err(|_| Error::InvalidArgument(format!("too many threads for an executor: {threads}")))?;
        let name = c_string(name, "the name of the threads")?;
        // SAFETY: the name is NUL-terminated and the out-pointer is valid for writes.
        let handle = Handle::create(|t, out| unsafe { new(t, threads, name.as_ptr(), out) })?;
        Ok(Executor::from_handle(handle))
    }

    /// The executor of a handle that the SDK returned.
    pub(crate) fn from_handle(handle: Handle<sys::dxfg_v4_executor_t>) -> Executor {
        Executor {
            handle: Arc::new(handle),
        }
    }
}

/// An executor that runs the tasks of an endpoint, listener calls included, only inside
/// [`process_all_pending`](Self::process_all_pending), on the calling thread.
///
/// It is an [`Executor`]: pass it where one is taken. Useful for tests and for applications that deliver events on a
/// thread of their own. Clones share one executor.
///
/// [`system::shutdown`](crate::system::shutdown) waits for the tasks of every endpoint: while it runs, another thread
/// must call `process_all_pending`, or it blocks.
#[derive(Clone)]
pub struct ManualExecutor {
    executor: Executor,
}

impl ManualExecutor {
    /// Creates an executor (`ExecutorBaseOnConcurrentLinkedQueue`).
    pub fn new() -> Result<Self> {
        // SAFETY: the out-pointer is valid for writes.
        let handle = Handle::create(|t, out| unsafe { sys::dxfg_v4_ExecutorBaseOnConcurrentLinkedQueue_new(t, out) })?;
        Ok(ManualExecutor {
            executor: Executor::from_handle(handle),
        })
    }

    /// Runs the queued tasks on the calling thread until the queue is empty, tasks added meanwhile included.
    pub fn process_all_pending(&self) -> Result<()> {
        let executor = self.executor.handle.as_ptr();
        // SAFETY: the executor is live while `self` lives.
        call(|t| unsafe { sys::dxfg_v4_ExecutorBaseOnConcurrentLinkedQueue_processAllPendingTasks(t, executor) })
    }
}

impl Deref for ManualExecutor {
    type Target = Executor;

    fn deref(&self) -> &Executor {
        &self.executor
    }
}

impl From<ManualExecutor> for Executor {
    fn from(executor: ManualExecutor) -> Executor {
        executor.executor
    }
}
