//! An executor whose tasks run when the application asks (KTD11).

use std::sync::Arc;

use dxfeed_graal_sys as sys;

use crate::error::Result;
use crate::handle::Handle;
use crate::isolate::call;

/// An executor that runs the tasks of an endpoint, listener calls included, only inside
/// [`process_all_pending`](Self::process_all_pending), on the calling thread.
///
/// Pass it to [`EndpointBuilder::with_executor`](crate::EndpointBuilder::with_executor). Useful for tests and for
/// applications that deliver events on a thread of their own. Clones share one executor.
#[derive(Clone)]
pub struct ManualExecutor {
    pub(crate) handle: Arc<Handle<sys::dxfg_v4_executor_t>>,
}

impl ManualExecutor {
    /// Creates an executor (`ExecutorBaseOnConcurrentLinkedQueue`).
    pub fn new() -> Result<Self> {
        // SAFETY: the out-pointer is valid for writes.
        let handle = Handle::create(|t, out| unsafe { sys::dxfg_v4_ExecutorBaseOnConcurrentLinkedQueue_new(t, out) })?;
        Ok(ManualExecutor {
            handle: Arc::new(handle),
        })
    }

    /// Runs the queued tasks on the calling thread until the queue is empty, tasks added meanwhile included.
    pub fn process_all_pending(&self) -> Result<()> {
        let executor = self.handle.as_ptr();
        // SAFETY: the executor is live while `self` lives.
        call(|t| unsafe { sys::dxfg_v4_ExecutorBaseOnConcurrentLinkedQueue_processAllPendingTasks(t, executor) })
    }
}
