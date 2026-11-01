//! Listeners of events, of endpoint states, of subscription changes and of instrument profiles: their C trampolines,
//! the release thread and the lists of listeners added to an SDK object.

use std::cell::Cell;
use std::ffi::c_void;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, OnceLock};

use dxfeed_graal_sys as sys;

use crate::endpoint::State;
use crate::error::{Error, Result};
use crate::event::{Events, IndexedEventSource};
use crate::handle::{Handle, SdkHandle};
use crate::ipf::InstrumentProfile;
use crate::ipf::live::ConnectionState;
use crate::isolate::{self, CallbackScope};
use crate::observable::ObservableSubscriptionChangeListener;
use crate::subscription::ListenerId;
use crate::symbol::Symbol;

/// The closure of an event listener.
pub(crate) type EventClosure = dyn FnMut(Events<'_>) + Send;

/// The closure of an endpoint state listener: it receives the old and the new state.
pub(crate) type StateClosure = dyn FnMut(State, State) + Send;

/// The closure of a change listener of a subscription.
pub(crate) type ChangeClosure = dyn ObservableSubscriptionChangeListener;

/// The closure of an update listener of an instrument profile collector.
pub(crate) type UpdateClosure = dyn FnMut(&[InstrumentProfile]) + Send;

/// The closure of a state listener of an instrument profile connection: it receives the old and the new state.
pub(crate) type IpfStateClosure = dyn FnMut(ConnectionState, ConnectionState) + Send;

/// The closure of a listener of a transactional model: it receives the source, the events and whether they are a
/// snapshot.
pub(crate) type TxClosure = dyn FnMut(IndexedEventSource, Events<'_>, bool) + Send;

/// The closure of a listener of an indexed event transactional model: it receives the events and whether they are a
/// snapshot.
pub(crate) type EventTxClosure = dyn FnMut(Events<'_>, bool) + Send;

/// The closure of a listener of an order book model: it is called after the book has changed.
pub(crate) type OrderBookClosure = dyn FnMut() + Send;

/// A listener of a subscription.
pub(crate) type EventListener = Listener<sys::dxfg_v4_feed_event_listener_t, EventClosure>;

/// A state listener of an endpoint.
pub(crate) type StateListener = Listener<sys::dxfg_v4_endpoint_state_change_listener_t, StateClosure>;

/// A change listener of a subscription.
pub(crate) type ChangeListener = Listener<sys::dxfg_v4_observable_subscription_change_listener_t, ChangeClosure>;

/// An update listener of an instrument profile collector.
pub(crate) type UpdateListener = Listener<sys::dxfg_v4_ipf_update_listener_t, UpdateClosure>;

/// A state listener of an instrument profile connection.
pub(crate) type IpfStateListener = Listener<sys::dxfg_v4_ipf_connection_state_change_listener_t, IpfStateClosure>;

/// A listener of a transactional model.
pub(crate) type TxListener = Listener<sys::dxfg_v4_tx_model_listener_t, TxClosure>;

/// A listener of an indexed event transactional model.
pub(crate) type EventTxListener = Listener<sys::dxfg_v4_indexed_event_tx_model_listener_t, EventTxClosure>;

/// A listener of an order book model.
pub(crate) type OrderBookListener = Listener<sys::dxfg_v4_order_book_model_listener_t, OrderBookClosure>;

/// A listener of a list of a model: it receives all the events of the list.
pub(crate) type ListListener = Listener<sys::dxfg_v4_observable_list_model_listener_t, EventClosure>;

/// The change listeners of a subscription.
pub(crate) type ChangeListeners = Listeners<sys::dxfg_v4_observable_subscription_change_listener_t, ChangeClosure>;

/// The update listeners of an instrument profile collector.
pub(crate) type UpdateListeners = Listeners<sys::dxfg_v4_ipf_update_listener_t, UpdateClosure>;

/// The state listeners of an instrument profile connection.
pub(crate) type IpfStateListeners = Listeners<sys::dxfg_v4_ipf_connection_state_change_listener_t, IpfStateClosure>;

/// The listeners of an order book model.
pub(crate) type OrderBookListeners = Listeners<sys::dxfg_v4_order_book_model_listener_t, OrderBookClosure>;

/// The listeners of a list of a model.
pub(crate) type ListListeners = Listeners<sys::dxfg_v4_observable_list_model_listener_t, EventClosure>;

/// What a trampoline needs; shared by the owner of the listener, the SDK (`user_data`) and running calls.
struct ListenerState<C: ?Sized> {
    closure: Mutex<Box<C>>,
    /// The id of the thread running the closure, `0` when none.
    running: AtomicU64,
    /// Whether a close was deferred during the running call (see [`Nested::Defer`]).
    close_deferred: AtomicBool,
    /// Whether a nested call asked for one more call after the running one (see [`Nested::Repeat`]).
    repeat: AtomicBool,
}

/// A listener added to an SDK object: its SDK handle and the SDK's strong reference to its state.
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

impl UpdateListener {
    pub(crate) fn new(closure: Box<UpdateClosure>) -> Result<Self> {
        Self::create(closure, |user_data| {
            // SAFETY: as for event listeners, with `update_trampoline`.
            Handle::create(|t, out| unsafe {
                sys::dxfg_v4_InstrumentProfileUpdateListener_new(t, Some(update_trampoline), user_data, out)
            })
        })
    }
}

impl IpfStateListener {
    pub(crate) fn new(closure: Box<IpfStateClosure>) -> Result<Self> {
        Self::create(closure, |user_data| {
            // SAFETY: as for event listeners, with `ipf_state_trampoline`.
            Handle::create(|t, out| unsafe {
                sys::dxfg_v4_IpfPropertyChangeListener_new(t, Some(ipf_state_trampoline), user_data, out)
            })
        })
    }
}

impl TxListener {
    pub(crate) fn new(closure: Box<TxClosure>) -> Result<Self> {
        Self::create(closure, |user_data| {
            // SAFETY: as for event listeners, with `tx_trampoline`.
            Handle::create(|t, out| unsafe { sys::dxfg_v4_TxModelListener_new(t, Some(tx_trampoline), user_data, out) })
        })
    }
}

impl EventTxListener {
    pub(crate) fn new(closure: Box<EventTxClosure>) -> Result<Self> {
        Self::create(closure, |user_data| {
            // SAFETY: as for event listeners, with `event_tx_trampoline`.
            Handle::create(|t, out| unsafe {
                sys::dxfg_v4_IndexedEventTxModel_Listener_new(t, Some(event_tx_trampoline), user_data, out)
            })
        })
    }
}

impl OrderBookListener {
    pub(crate) fn new(closure: Box<OrderBookClosure>) -> Result<Self> {
        Self::create(closure, |user_data| {
            // SAFETY: as for event listeners, with `order_book_trampoline`.
            Handle::create(|t, out| unsafe {
                sys::dxfg_v4_OrderBookModelListener_new(t, Some(order_book_trampoline), user_data, out)
            })
        })
    }
}

impl ListListener {
    pub(crate) fn new(closure: Box<EventClosure>) -> Result<Self> {
        Self::create(closure, |user_data| {
            // SAFETY: as for event listeners, with `list_trampoline`.
            Handle::create(|t, out| unsafe {
                sys::dxfg_v4_ObservableListModelListener_new(t, Some(list_trampoline), user_data, out)
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
            repeat: AtomicBool::new(false),
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
        let released = handle.release();
        let ok = released.is_ok();
        crate::error_manager::report_release("JavaObjectHandler", released);
        if ok && !isolate::exiting() {
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
        dispatch::<EventClosure>(
            thread,
            user_data,
            "DXFeedSubscription",
            "an event listener",
            Nested::Warn,
            |closure| closure(Events::new(events)),
        )
    };
}

unsafe extern "C" fn tx_trampoline(
    thread: *mut sys::graal_isolatethread_t,
    source: *mut sys::dxfg_indexed_event_source_t,
    events: *mut sys::dxfg_event_type_list,
    is_snapshot: i32,
    user_data: *mut c_void,
) {
    // SAFETY: the SDK keeps the source and its name alive until this call returns.
    let source = unsafe { source.as_ref() }
        .map(|source| unsafe { IndexedEventSource::from_sys(source) })
        .unwrap_or_default();
    // SAFETY: `user_data` is the state of a transactional model listener, and the SDK keeps the list alive until this
    // call returns.
    unsafe {
        dispatch::<TxClosure>(
            thread,
            user_data,
            "TxModel",
            "a transactional model listener",
            Nested::Warn,
            |closure| closure(source, Events::new(events), is_snapshot != 0),
        )
    };
}

unsafe extern "C" fn event_tx_trampoline(
    thread: *mut sys::graal_isolatethread_t,
    events: *mut sys::dxfg_event_type_list,
    is_snapshot: i32,
    user_data: *mut c_void,
) {
    // SAFETY: `user_data` is the state of an indexed event transactional model listener, and the SDK keeps the list
    // alive until this call returns.
    unsafe {
        dispatch::<EventTxClosure>(
            thread,
            user_data,
            "IndexedEventTxModel",
            "an indexed event transactional model listener",
            Nested::Warn,
            |closure| closure(Events::new(events), is_snapshot != 0),
        )
    };
}

unsafe extern "C" fn order_book_trampoline(
    thread: *mut sys::graal_isolatethread_t,
    _model: *mut sys::dxfg_order_book_model_t,
    user_data: *mut c_void,
) {
    // SAFETY: `user_data` is the state of an order book model listener.
    unsafe {
        dispatch::<OrderBookClosure>(
            thread,
            user_data,
            "OrderBookModel",
            "an order book model listener",
            Nested::Repeat(|closure: &mut OrderBookClosure| closure()),
            |closure| closure(),
        )
    };
}

unsafe extern "C" fn list_trampoline(
    thread: *mut sys::graal_isolatethread_t,
    events: *mut sys::dxfg_event_type_list,
    user_data: *mut c_void,
) {
    // SAFETY: `user_data` is the state of a list listener, and the SDK keeps the list alive until this call returns.
    unsafe {
        dispatch::<EventClosure>(
            thread,
            user_data,
            "ObservableListModel",
            "a list listener",
            Nested::Skip,
            |closure| closure(Events::new(events)),
        )
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
            "DXEndpoint",
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
            "ObservableSubscription",
            "a subscription change listener",
            Nested::WarnThenClose(close_change_listener),
            |listener| listener.symbols_added(&Symbol::list_from_sys(symbols, "ObservableSubscription")),
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
            "ObservableSubscription",
            "a subscription change listener",
            Nested::WarnThenClose(close_change_listener),
            |listener| listener.symbols_removed(&Symbol::list_from_sys(symbols, "ObservableSubscription")),
        )
    };
}

unsafe extern "C" fn subscription_closed_trampoline(thread: *mut sys::graal_isolatethread_t, user_data: *mut c_void) {
    // SAFETY: `user_data` is the state of a change listener.
    unsafe {
        dispatch::<ChangeClosure>(
            thread,
            user_data,
            "ObservableSubscription",
            "a subscription change listener",
            Nested::Defer,
            close_change_listener,
        )
    };
}

fn close_change_listener(listener: &mut ChangeClosure) {
    listener.subscription_closed();
}

unsafe extern "C" fn update_trampoline(
    thread: *mut sys::graal_isolatethread_t,
    profiles: *mut sys::dxfg_iterable_ip_t,
    user_data: *mut c_void,
) {
    // SAFETY: `user_data` is the state of an update listener, and the SDK keeps the iterator alive until this call
    // returns.
    unsafe {
        dispatch::<UpdateClosure>(
            thread,
            user_data,
            "InstrumentProfileCollector",
            "an instrument profile update listener",
            Nested::Warn,
            |closure| match crate::ipf::live::collect(profiles) {
                Ok(profiles) => closure(&profiles),
                Err(e) => crate::ErrorManager::report_error("InstrumentProfileCollector", &e),
            },
        )
    };
}

unsafe extern "C" fn ipf_state_trampoline(
    thread: *mut sys::graal_isolatethread_t,
    old_state: sys::dxfg_ipf_connection_state_t,
    new_state: sys::dxfg_ipf_connection_state_t,
    user_data: *mut c_void,
) {
    // SAFETY: `user_data` is the state of a connection state listener.
    unsafe {
        dispatch::<IpfStateClosure>(
            thread,
            user_data,
            "InstrumentProfileConnection",
            "an instrument profile connection state listener",
            Nested::Warn,
            |closure| match (
                ConnectionState::from_sys(old_state),
                ConnectionState::from_sys(new_state),
            ) {
                (Ok(old_state), Ok(new_state)) => closure(old_state, new_state),
                _ => crate::ErrorManager::report("InstrumentProfileConnection", "skipped an unknown connection state"),
            },
        )
    };
}

/// The listeners added to an SDK object, by id. Dropping them releases those left, which their object no longer
/// calls: the owner removes them first, or the object is closed.
pub(crate) struct Listeners<H: SdkHandle + 'static, C: ?Sized + Send + 'static> {
    listeners: Mutex<Vec<(ListenerId, Listener<H, C>)>>,
    /// What the listeners are, for errors, such as `a change listener of this subscription`.
    kind: &'static str,
}

impl<H: SdkHandle + 'static, C: ?Sized + Send + 'static> Listeners<H, C> {
    pub(crate) fn new(kind: &'static str) -> Self {
        Listeners {
            listeners: Mutex::new(Vec::new()),
            kind,
        }
    }

    /// Adds `listener` to its object with `add` and keeps it, or releases it if that fails.
    pub(crate) fn add(&self, listener: Listener<H, C>, add: impl FnOnce(*mut H) -> Result<()>) -> Result<ListenerId> {
        if let Err(e) = add(listener.as_ptr()) {
            listener.release();
            return Err(e);
        }
        let id = ListenerId::next();
        crate::lock(&self.listeners).push((id, listener));
        Ok(id)
    }

    /// Removes the listener `id` from its object with `remove`, then releases it.
    pub(crate) fn remove(&self, id: ListenerId, remove: impl FnOnce(*mut H) -> Result<()>) -> Result<()> {
        let listener = {
            let mut listeners = crate::lock(&self.listeners);
            let index = listeners
                .iter()
                .position(|(i, _)| *i == id)
                .ok_or_else(|| Error::InvalidArgument(format!("{id:?} is not {}", self.kind)))?;
            listeners.swap_remove(index).1
        };
        let removed = remove(listener.as_ptr());
        listener.release();
        removed
    }

    /// Removes every listener with `remove` and releases it, ignoring errors.
    pub(crate) fn remove_all(&self, component: &str, remove: impl Fn(*mut H) -> Result<()>) {
        let listeners = std::mem::take(&mut *crate::lock(&self.listeners));
        for (_, listener) in listeners {
            crate::error_manager::report_release(component, remove(listener.as_ptr()));
            listener.release();
        }
    }
}

impl<H: SdkHandle + 'static, C: ?Sized + Send + 'static> Drop for Listeners<H, C> {
    fn drop(&mut self) {
        let listeners = std::mem::take(&mut *crate::lock(&self.listeners));
        for (_, listener) in listeners {
            listener.release();
        }
    }
}

/// What [`dispatch`] does with a call nested in a running call of the same listener on its thread.
enum Nested<C: ?Sized> {
    /// Skips it and reports it to `ErrorManager`: the SDK serializes the calls of a listener, so this is a guard
    /// against a deadlock on the closure lock, not an expected path.
    Warn,
    /// Skips it and reports it to `ErrorManager`, and after the running call, runs the close deferred during it.
    WarnThenClose(fn(&mut C)),
    /// Defers it until the running call returns: the close of a change listener that removes itself or closes its
    /// subscription or endpoint in its own call.
    Defer,
    /// Calls the closure once more with `again` after the running call returns, however many nested calls came: a
    /// listener of a model that changes the model in its own call, which Java notifies on the same thread.
    Repeat(fn(&mut C)),
    /// Skips it: a listener of a list whose events are only valid during the nested call, which the running call
    /// can read with `toArray`.
    Skip,
}

/// Runs the closure of the listener whose state is `user_data` with `call`, catching panics; `component` and
/// `what` name the listener in the reports to [`ErrorManager`](crate::ErrorManager), and `nested` tells what to
/// do with a nested call.
///
/// # Safety
///
/// `user_data` must be the state of a live listener whose closure type is `C`.
unsafe fn dispatch<C: ?Sized>(
    thread: *mut sys::graal_isolatethread_t,
    user_data: *mut c_void,
    component: &str,
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
                Nested::Repeat(_) => state.repeat.store(true, Ordering::Release),
                Nested::Skip => {}
                Nested::Warn | Nested::WarnThenClose(_) => crate::ErrorManager::report(
                    component,
                    format!("skipped a nested call of {what} on the thread that runs it"),
                ),
            }
            return;
        }
        let mut closure = crate::lock(&state.closure);
        state.running.store(me, Ordering::Release);
        crate::error_manager::report_panic(
            component,
            what,
            panic::catch_unwind(AssertUnwindSafe(|| call(&mut **closure))),
        );
        if let Nested::Repeat(again) = nested {
            while state.repeat.swap(false, Ordering::AcqRel) {
                crate::error_manager::report_panic(
                    component,
                    what,
                    panic::catch_unwind(AssertUnwindSafe(|| again(&mut **closure))),
                );
            }
        }
        if let Nested::WarnThenClose(close) = nested
            && state.close_deferred.swap(false, Ordering::AcqRel)
        {
            crate::error_manager::report_panic(
                component,
                what,
                panic::catch_unwind(AssertUnwindSafe(|| close(&mut **closure))),
            );
        }
        state.running.store(0, Ordering::Release);
    }));
}
