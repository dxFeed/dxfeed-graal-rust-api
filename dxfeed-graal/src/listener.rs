//! Listeners of events, of endpoint states and of subscription changes: their C trampolines and the release thread.

use std::cell::Cell;
use std::ffi::c_void;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, OnceLock};

use dxfeed_graal_sys as sys;

use crate::endpoint::State;
use crate::error::Result;
use crate::event::Events;
use crate::handle::{Handle, SdkHandle};
use crate::isolate::{self, CallbackScope};
use crate::observable::ObservableSubscriptionChangeListener;
use crate::symbol::Symbol;

/// The closure of an event listener.
pub(crate) type EventClosure = dyn FnMut(Events<'_>) + Send;

/// The closure of an endpoint state listener: it receives the old and the new state.
pub(crate) type StateClosure = dyn FnMut(State, State) + Send;

/// The closure of a change listener of a subscription.
pub(crate) type ChangeClosure = dyn ObservableSubscriptionChangeListener;

/// A listener of a subscription.
pub(crate) type EventListener = Listener<sys::dxfg_v4_feed_event_listener_t, EventClosure>;

/// A state listener of an endpoint.
pub(crate) type StateListener = Listener<sys::dxfg_v4_endpoint_state_change_listener_t, StateClosure>;

/// A change listener of a subscription.
pub(crate) type ChangeListener = Listener<sys::dxfg_v4_observable_subscription_change_listener_t, ChangeClosure>;

/// What a trampoline needs; shared by the owner of the listener, the SDK (`user_data`) and running calls.
struct ListenerState<C: ?Sized> {
    closure: Mutex<Box<C>>,
    /// The id of the thread running the closure, `0` when none.
    running: AtomicU64,
    /// Whether a close was deferred during the running call (see [`Nested::Defer`]).
    close_deferred: AtomicBool,
}

/// A listener added to a subscription or an endpoint: its SDK handle and the SDK's strong reference to its state.
pub(crate) struct Listener<H: SdkHandle, C: ?Sized> {
    handle: Handle<H>,
    state: *const ListenerState<C>,
}

// SAFETY: the handle is Send, and the state is an `Arc` of a Sync value whose closure is Send.
unsafe impl<H: SdkHandle, C: ?Sized + Send> Send for Listener<H, C> {}

impl EventListener {
    pub(crate) fn new(closure: Box<EventClosure>) -> Result<Self> {
        Self::create(closure, |user_data| {
            // SAFETY: `event_trampoline` matches the listener function type and `user_data` stays valid until the
            // listener is released (see `release`). The out-pointer is valid for writes.
            Handle::create(|t, out| unsafe {
                sys::dxfg_v4_DXFeedEventListener_new(t, Some(event_trampoline), user_data, out)
            })
        })
    }
}

impl StateListener {
    pub(crate) fn new(closure: Box<StateClosure>) -> Result<Self> {
        Self::create(closure, |user_data| {
            // SAFETY: as for event listeners, with `state_trampoline`.
            Handle::create(|t, out| unsafe {
                sys::dxfg_v4_PropertyChangeListener_new(t, Some(state_trampoline), user_data, out)
            })
        })
    }
}

impl ChangeListener {
    pub(crate) fn new(closure: Box<ChangeClosure>) -> Result<Self> {
        Self::create(closure, |user_data| {
            // SAFETY: as for event listeners, with the three change trampolines, which the SDK calls without a check
            // for NULL.
            Handle::create(|t, out| unsafe {
                sys::dxfg_v4_ObservableSubscriptionChangeListener_new(
                    t,
                    Some(symbols_added_trampoline),
                    Some(symbols_removed_trampoline),
                    Some(subscription_closed_trampoline),
                    user_data,
                    out,
                )
            })
        })
    }
}

impl<H: SdkHandle + 'static, C: ?Sized + Send + 'static> Listener<H, C> {
    /// Creates the listener with `new`, which gets the `user_data` of the trampoline.
    fn create(closure: Box<C>, new: impl FnOnce(*mut c_void) -> Result<Handle<H>>) -> Result<Self> {
        let state = Arc::into_raw(Arc::new(ListenerState {
            closure: Mutex::new(closure),
            running: AtomicU64::new(0),
            close_deferred: AtomicBool::new(false),
        }));
        match new(state as *mut c_void) {
            Ok(handle) => Ok(Listener { handle, state }),
            Err(e) => {
                // SAFETY: the SDK did not take the reference, so it is ours to drop.
                drop(unsafe { Arc::from_raw(state) });
                Err(e)
            }
        }
    }

    pub(crate) fn as_ptr(&self) -> *mut H {
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

unsafe extern "C" fn event_trampoline(
    thread: *mut sys::graal_isolatethread_t,
    events: *mut sys::dxfg_event_type_list,
    user_data: *mut c_void,
) {
    // SAFETY: `user_data` is the state of an event listener, and the SDK keeps the list alive until this call returns.
    unsafe {
        dispatch::<EventClosure>(thread, user_data, "an event listener", Nested::Warn, |closure| {
            closure(Events::new(events))
        })
    };
}

unsafe extern "C" fn state_trampoline(
    thread: *mut sys::graal_isolatethread_t,
    old_state: sys::dxfg_endpoint_state_t,
    new_state: sys::dxfg_endpoint_state_t,
    user_data: *mut c_void,
) {
    // A code this crate does not know cannot be reported; the SDK has no other states.
    let (Ok(old_state), Ok(new_state)) = (State::from_sys(old_state), State::from_sys(new_state)) else {
        return;
    };
    // SAFETY: `user_data` is the state of a state listener.
    unsafe {
        dispatch::<StateClosure>(
            thread,
            user_data,
            "an endpoint state listener",
            Nested::Warn,
            |closure| closure(old_state, new_state),
        )
    };
}

unsafe extern "C" fn symbols_added_trampoline(
    thread: *mut sys::graal_isolatethread_t,
    symbols: *mut sys::dxfg_symbol_list,
    user_data: *mut c_void,
) {
    // SAFETY: `user_data` is the state of a change listener, and the SDK keeps the list alive until this call returns.
    unsafe {
        dispatch::<ChangeClosure>(
            thread,
            user_data,
            "a subscription change listener",
            Nested::WarnThenClose(close_change_listener),
            |listener| listener.symbols_added(&Symbol::list_from_sys(symbols)),
        )
    };
}

unsafe extern "C" fn symbols_removed_trampoline(
    thread: *mut sys::graal_isolatethread_t,
    symbols: *mut sys::dxfg_symbol_list,
    user_data: *mut c_void,
) {
    // SAFETY: as for the added symbols.
    unsafe {
        dispatch::<ChangeClosure>(
            thread,
            user_data,
            "a subscription change listener",
            Nested::WarnThenClose(close_change_listener),
            |listener| listener.symbols_removed(&Symbol::list_from_sys(symbols)),
        )
    };
}

unsafe extern "C" fn subscription_closed_trampoline(thread: *mut sys::graal_isolatethread_t, user_data: *mut c_void) {
    // SAFETY: `user_data` is the state of a change listener.
    unsafe {
        dispatch::<ChangeClosure>(
            thread,
            user_data,
            "a subscription change listener",
            Nested::Defer,
            close_change_listener,
        )
    };
}

fn close_change_listener(listener: &mut ChangeClosure) {
    listener.subscription_closed();
}

/// What [`dispatch`] does with a call nested in a running call of the same listener on its thread.
enum Nested<C: ?Sized> {
    /// Skips it with a warning: the SDK serializes the calls of a listener, so this is a guard against a deadlock on
    /// the closure lock, not an expected path.
    Warn,
    /// Skips it with a warning, and after the running call, runs the close deferred during it.
    WarnThenClose(fn(&mut C)),
    /// Defers it until the running call returns: the close of a change listener that removes itself or closes its
    /// subscription or endpoint in its own call.
    Defer,
}

/// Runs the closure of the listener whose state is `user_data` with `call`, catching panics; `what` names the
/// listener in messages, and `nested` tells what to do with a nested call.
///
/// # Safety
///
/// `user_data` must be the state of a live listener whose closure type is `C`.
unsafe fn dispatch<C: ?Sized>(
    thread: *mut sys::graal_isolatethread_t,
    user_data: *mut c_void,
    what: &str,
    nested: Nested<C>,
    call: impl FnOnce(&mut C),
) {
    if isolate::exiting() {
        return;
    }
    let state = user_data as *const ListenerState<C>;
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
            match nested {
                Nested::Defer => state.close_deferred.store(true, Ordering::Release),
                Nested::Warn | Nested::WarnThenClose(_) => {
                    eprintln!("dxfeed-graal: skipped a nested call of {what} on the thread that runs it")
                }
            }
            return;
        }
        let mut closure = crate::lock(&state.closure);
        state.running.store(me, Ordering::Release);
        report(what, panic::catch_unwind(AssertUnwindSafe(|| call(&mut **closure))));
        if let Nested::WarnThenClose(close) = nested
            && state.close_deferred.swap(false, Ordering::AcqRel)
        {
            report(what, panic::catch_unwind(AssertUnwindSafe(|| close(&mut **closure))));
        }
        state.running.store(0, Ordering::Release);
    }));
}

/// Prints the panic of a call of a listener named by `what`, if it panicked.
fn report(what: &str, result: std::thread::Result<()>) {
    if let Err(payload) = result {
        let message = payload
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
            .unwrap_or("unknown panic");
        eprintln!("dxfeed-graal: {what} panicked: {message}");
    }
}
