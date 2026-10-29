//! Event listeners: the C trampoline and the release thread.

use std::cell::Cell;
use std::ffi::c_void;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, OnceLock};

use dxfeed_graal_sys as sys;

use crate::error::Result;
use crate::event::Events;
use crate::handle::Handle;
use crate::isolate::{self, CallbackScope};

/// The closure of a listener.
pub(crate) type Closure = Box<dyn FnMut(Events<'_>) + Send>;

/// What the trampoline needs; shared by the subscription, the SDK (`user_data`) and running calls.
pub(crate) struct ListenerState {
    closure: Mutex<Closure>,
    /// The id of the thread running the closure, `0` when none.
    running: AtomicU64,
}

/// A listener added to a subscription: its SDK handle and the SDK's strong reference to its state.
pub(crate) struct Listener {
    handle: Handle<sys::dxfg_v4_feed_event_listener_t>,
    state: *const ListenerState,
}

// SAFETY: the handle is Send, and the state is an `Arc` of a Sync value whose closure is Send.
unsafe impl Send for Listener {}

impl Listener {
    pub(crate) fn new(closure: Closure) -> Result<Self> {
        let state = Arc::into_raw(Arc::new(ListenerState {
            closure: Mutex::new(closure),
            running: AtomicU64::new(0),
        }));
        let user_data = state as *mut c_void;
        // SAFETY: `trampoline` matches the listener function type and `user_data` stays valid until the listener is
        // released (see `release`). The out-pointer is valid for writes.
        let handle = Handle::create(|t, out| unsafe {
            sys::dxfg_v4_DXFeedEventListener_new(t, Some(trampoline), user_data, out)
        });
        match handle {
            Ok(handle) => Ok(Listener { handle, state }),
            Err(e) => {
                // SAFETY: the SDK did not take the reference, so it is ours to drop.
                drop(unsafe { Arc::from_raw(state) });
                Err(e)
            }
        }
    }

    pub(crate) fn as_ptr(&self) -> *mut sys::dxfg_v4_feed_event_listener_t {
        self.handle.as_ptr()
    }

    /// Releases the listener: no call of it runs or starts afterwards, except on the calling thread, and its state is
    /// freed once the last running call ends. Inside a callback this goes to the release thread, because releasing
    /// waits for calls on other threads, which may be waiting for us.
    pub(crate) fn release(self) {
        if isolate::in_callback() {
            release_thread(Box::new(move || self.release_now()));
        } else {
            self.release_now();
        }
    }

    fn release_now(self) {
        let Listener { handle, state } = self;
        // While the process is exiting, the release reports success without calling the SDK, so `exiting` is checked
        // after it: a release that reached the SDK just before the exit started is then leaked too, which is harmless.
        if handle.release().is_ok() && !isolate::exiting() {
            // SAFETY: the SDK released the handle, so it no longer calls the trampoline with `state`; its reference
            // is ours to drop.
            drop(unsafe { Arc::from_raw(state) });
        }
        // If the release failed or the process is exiting, the SDK may still call the trampoline: its reference is
        // leaked on purpose.
    }
}

type Job = Box<dyn FnOnce() + Send>;

static RELEASE_THREAD: OnceLock<Sender<Job>> = OnceLock::new();

/// Runs `job` on the crate's release thread, started on first use.
fn release_thread(job: Job) {
    let sender = RELEASE_THREAD.get_or_init(|| {
        let (sender, receiver) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("dxfeed-graal-release".into())
            .spawn(move || {
                for job in receiver {
                    if isolate::exiting() {
                        break;
                    }
                    job();
                }
            })
            .expect("cannot start the dxfeed-graal release thread");
        sender
    });
    // The receiver lives as long as the process, so sending cannot fail.
    let _ = sender.send(job);
}

/// Waits until the release thread has run the jobs sent before this call, if it was ever started.
pub(crate) fn drain_release_thread() {
    if RELEASE_THREAD.get().is_none() {
        return;
    }
    let (done, wait) = mpsc::channel();
    release_thread(Box::new(move || {
        let _ = done.send(());
    }));
    // An error means the thread stopped because the process is exiting: nothing to wait for.
    let _ = wait.recv();
}

/// A process-unique id of the calling thread, never `0`.
fn thread_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    thread_local! {
        static ID: Cell<u64> = const { Cell::new(0) };
    }
    ID.with(|id| {
        if id.get() == 0 {
            id.set(NEXT.fetch_add(1, Ordering::Relaxed));
        }
        id.get()
    })
}

unsafe extern "C" fn trampoline(
    thread: *mut sys::graal_isolatethread_t,
    events: *mut sys::dxfg_event_type_list,
    user_data: *mut c_void,
) {
    if isolate::exiting() {
        return;
    }
    let state = user_data as *const ListenerState;
    // SAFETY: the SDK holds a strong reference until the listener is released, which waits for this call; this call
    // takes its own for its duration, so dropping the subscription from the closure cannot free the state under it.
    let state = unsafe {
        Arc::increment_strong_count(state);
        Arc::from_raw(state)
    };
    let _ = panic::catch_unwind(AssertUnwindSafe(|| {
        let _scope = CallbackScope::enter(thread);
        let me = thread_id();
        if state.running.load(Ordering::Acquire) == me {
            // A nested call on the thread that runs this listener. The SDK serializes the calls of a subscription, so
            // this is a guard against a deadlock on the closure lock, not an expected path.
            eprintln!("dxfeed-graal: skipped a nested call of an event listener on the thread that runs it");
            return;
        }
        let mut closure = crate::lock(&state.closure);
        state.running.store(me, Ordering::Release);
        // SAFETY: the SDK keeps the list alive until this call returns.
        let events = unsafe { Events::new(events) };
        let result = panic::catch_unwind(AssertUnwindSafe(|| closure(events)));
        state.running.store(0, Ordering::Release);
        if let Err(payload) = result {
            let message = payload
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("unknown panic");
            eprintln!("dxfeed-graal: an event listener panicked: {message}");
        }
    }));
}
