//! Errors without a caller (`ErrorManager` of the C++ API).

use std::cell::Cell;
use std::collections::VecDeque;
use std::fmt;
use std::panic::{self, AssertUnwindSafe, Location};
use std::sync::{Arc, Mutex};
use std::thread::{self, ThreadId};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::Error;

/// An error without a caller, reported to [`ErrorManager`] (`ErrorInfo`). A value: copies do not change when
/// [`ErrorManager`] gets new errors.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ErrorInfo {
    /// The number of the error, from 1, increasing in the order of the reports.
    pub id: u64,
    /// The time of the report, in milliseconds since the Unix epoch.
    pub time: i64,
    /// The thread that reported the error.
    pub thread_id: ThreadId,
    /// The part of the API that reported the error, for example `DXFeedSubscription`.
    pub component: String,
    /// The source file and the line of the report, for example `listener.rs:42`.
    pub location: String,
    /// The message of the error; for a Java exception, the message of the exception.
    pub message: String,
    /// The class of the Java exception, if the error is one.
    pub java_class_name: Option<String>,
    /// The stack trace of the Java exception, if the error is one.
    pub java_stack_trace: Option<String>,
}

/// The number, the component, the location, the Java class (if any) and the message of the error in one line, as
/// `toString` of the C++ API: `#1 DXFeedSubscription (listener.rs:42): an event listener panicked: boom`.
impl fmt::Display for ErrorInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let class = match &self.java_class_name {
            Some(class) => format!("{class}: "),
            None => String::new(),
        };
        let line = format!(
            "#{} {} ({}): {class}{}",
            self.id, self.component, self.location, self.message
        );
        f.write_str(&line.replace(['\n', '\r'], " "))
    }
}

/// The handler of the errors.
type Handler = Arc<dyn Fn(&ErrorInfo) + Send + Sync>;

struct State {
    last_id: u64,
    capacity: usize,
    errors: VecDeque<ErrorInfo>,
    /// `None` for the default handler.
    handler: Option<Handler>,
}

static STATE: Mutex<State> = Mutex::new(State {
    last_id: 0,
    capacity: ErrorManager::DEFAULT_CAPACITY,
    errors: VecDeque::new(),
    handler: None,
});

thread_local! {
    /// Set while the handler runs on this thread: an error reported from the handler goes to the journal only.
    static IN_HANDLER: Cell<bool> = const { Cell::new(false) };
}

/// Receives the errors that have no caller (`ErrorManager`).
///
/// The API reports an error through one of two channels:
///
/// - **A call that fails** returns [`Err`](crate::Error) to its caller.
/// - **An error without a caller** goes to [`ErrorManager`]. Such an error happens in the code that the API runs on
///   its own: a panic of a listener (of the events of a subscription, of a state change of an
///   endpoint or of an instrument profile connection, of a change of a subscription, of instrument profiles of a
///   collector, of the events of a model), data of the SDK that the API cannot read, or a failed release of the native
///   data of the SDK when a value is dropped. From the start of the exit of the process (or of
///   [`system::shutdown`](crate::system::shutdown)) a failed release is not reported: the SDK is being closed.
///
/// [`ErrorManager`] passes each error to the handler and keeps the last errors in a journal:
///
/// - The default handler, [`ErrorManager::default_handler`], writes one line per error to `stderr`.
///   [`ErrorManager::set_handler`] replaces it, [`ErrorManager::reset_handler`] restores it.
/// - The handler is called on the thread where the error happened: a thread of the SDK for a listener, the timer
///   thread of the crate for a market depth model, the dropping thread for a failed release. No lock of
///   [`ErrorManager`] is held: the handler may read the journal and change the handler. Errors of several threads
///   call it at the same time. Keep it short: it delays
///   the thread that reported the error, for example the delivery of the events of a subscription. A panic of the
///   handler is caught and ignored; an error reported from inside the handler goes to the journal only.
/// - The handler of an error of a listener runs inside the call of that listener, as in the C++ API: like the
///   listener, it must not wait for a thread that removes the listener or drops its owner, or both wait forever.
/// - The journal keeps the last [`ErrorManager::get_capacity`] errors ([`ErrorManager::DEFAULT_CAPACITY`] by
///   default): [`ErrorManager::get_errors`], [`ErrorManager::get_last_error`], [`ErrorManager::clear_errors`].
///
/// ```no_run
/// use dxfeed_graal::ErrorManager;
///
/// ErrorManager::set_handler(|error| eprintln!("MyApp: {error}"));
/// ```
#[derive(Debug)]
pub enum ErrorManager {}

impl ErrorManager {
    /// The default capacity of the journal.
    pub const DEFAULT_CAPACITY: usize = 128;

    /// Reports an error: adds it to the journal and passes it to the handler (`report`). The location is the one of
    /// the caller.
    #[track_caller]
    pub fn report(component: &str, message: impl Into<String>) {
        dispatch(info(component, message.into(), None, None, Location::caller()));
    }

    /// Reports an error of a call (`reportException`): a Java exception gives its message, class and stack trace,
    /// another error its text. The location is the one of the caller.
    #[track_caller]
    pub fn report_error(component: &str, error: &Error) {
        let location = Location::caller();
        let info = match error {
            Error::Java(e) => info(
                component,
                e.get_message().to_owned(),
                Some(e.get_class_name()).filter(|c| !c.is_empty()).map(str::to_owned),
                Some(e.get_stack_trace()).filter(|s| !s.is_empty()).map(str::to_owned),
                location,
            ),
            _ => info(component, error.to_string(), None, None, location),
        };
        dispatch(info);
    }

    /// Replaces the handler of the errors (`setHandler`). A handler that does nothing turns the output off; the
    /// journal keeps the errors.
    #[doc(alias = "setHandler")]
    pub fn set_handler(handler: impl Fn(&ErrorInfo) + Send + Sync + 'static) {
        // The old handler is dropped after the lock: its drop may report an error.
        let old = crate::lock(&STATE).handler.replace(Arc::new(handler));
        drop(old);
    }

    /// Restores the default handler (`resetHandler`).
    #[doc(alias = "resetHandler")]
    pub fn reset_handler() {
        let old = crate::lock(&STATE).handler.take();
        drop(old);
    }

    /// The default handler (`defaultHandler`): writes the error as one line to `stderr`, prefixed with
    /// `[dxfeed-graal]`.
    #[doc(alias = "defaultHandler")]
    pub fn default_handler(error: &ErrorInfo) {
        eprintln!("[dxfeed-graal] {error}");
    }

    /// Sets the number of the last errors that the journal keeps (`setCapacity`); older errors are dropped at once if
    /// there are more. 0 turns the journal off.
    #[doc(alias = "setCapacity")]
    pub fn set_capacity(capacity: usize) {
        let mut state = crate::lock(&STATE);
        state.capacity = capacity;
        let excess = state.errors.len().saturating_sub(capacity);
        state.errors.drain(..excess);
    }

    /// The number of the last errors that the journal keeps (`getCapacity`).
    #[doc(alias = "getCapacity")]
    pub fn get_capacity() -> usize {
        crate::lock(&STATE).capacity
    }

    /// The errors of the journal, from the oldest (`getErrors`).
    #[doc(alias = "getErrors")]
    pub fn get_errors() -> Vec<ErrorInfo> {
        crate::lock(&STATE).errors.iter().cloned().collect()
    }

    /// The last error of the journal, if any (`getLastError`).
    #[doc(alias = "getLastError")]
    pub fn get_last_error() -> Option<ErrorInfo> {
        crate::lock(&STATE).errors.back().cloned()
    }

    /// Empties the journal (`clearErrors`); the numbers of the next errors go on from the last one.
    #[doc(alias = "clearErrors")]
    pub fn clear_errors() {
        crate::lock(&STATE).errors.clear();
    }
}

fn info(
    component: &str,
    message: String,
    java_class_name: Option<String>,
    java_stack_trace: Option<String>,
    location: &Location<'_>,
) -> ErrorInfo {
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
    let file = location.file();
    let file = file.rsplit(['/', '\\']).next().unwrap_or(file);
    ErrorInfo {
        id: 0,
        time,
        thread_id: thread::current().id(),
        component: component.to_owned(),
        location: format!("{file}:{}", location.line()),
        message,
        java_class_name,
        java_stack_trace,
    }
}

fn dispatch(mut info: ErrorInfo) {
    let handler = {
        let mut state = crate::lock(&STATE);
        state.last_id += 1;
        info.id = state.last_id;
        if state.capacity > 0 {
            if state.errors.len() == state.capacity {
                state.errors.pop_front();
            }
            state.errors.push_back(info.clone());
        }
        state.handler.clone()
    };
    // `IN_HANDLER` has no destructor, so it is available even on a thread being torn down.
    if !IN_HANDLER.with(|flag| flag.replace(true)) {
        call(handler.as_ref(), &info);
        IN_HANDLER.with(|flag| flag.set(false));
    }
}

fn call(handler: Option<&Handler>, info: &ErrorInfo) {
    // The handler is the last stop of an error: its own panic has nowhere to go.
    let _ = panic::catch_unwind(AssertUnwindSafe(|| match handler {
        Some(handler) => handler(info),
        None => ErrorManager::default_handler(info),
    }));
}

/// Reports the failed release of the native data of the SDK named by `component`, unless the process is exiting or
/// the SDK is shut down.
#[track_caller]
pub(crate) fn report_release(component: &str, result: crate::Result<()>) {
    if let Err(error) = result
        && !crate::isolate::exiting()
        && !crate::isolate::shut_down()
    {
        ErrorManager::report_error(component, &error);
    }
}

/// Reports the panic of a call of a listener named by `what`, if it panicked.
#[track_caller]
pub(crate) fn report_panic(component: &str, what: &str, result: thread::Result<()>) {
    if let Err(payload) = result {
        let message = payload
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
            .unwrap_or("unknown panic");
        ErrorManager::report(component, format!("{what} panicked: {message}"));
    }
}
