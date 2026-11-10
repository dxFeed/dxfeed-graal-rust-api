//! The log of the SDK (`InterceptableLogging`).
//!
//! The SDK writes its log to the console by default, or to the files that the `log.file` and `err.file` system
//! properties name. [`set_handler`] routes the log to the application instead: from the first call on, the SDK gives
//! every message to the handler and writes nothing to the console or to the files. The SDK takes one interceptor per
//! process and never removes it, so this cannot be undone; a later call only replaces the handler, and a handler that
//! does nothing turns the log off.
//!
//! ```no_run
//! use dxfeed_graal::logging::{self, LogLevel};
//!
//! # fn main() -> dxfeed_graal::Result<()> {
//! logging::set_handler(|record| println!("{:?} {}: {}", record.level, record.thread_name, record.message))?;
//! logging::set_log_level(LogLevel::Info)?;
//! # Ok(())
//! # }
//! ```

use std::cell::Cell;
use std::ffi::{c_char, c_void};
use std::panic::{self, AssertUnwindSafe};
use std::sync::{Arc, Mutex, OnceLock};

use dxfeed_graal_sys as sys;

use crate::error::{JavaException, Result};
use crate::handle::Handle;
use crate::isolate::{self, CallbackScope, call};

/// The level of a message of the log (`Level`), from the most detailed to none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LogLevel {
    /// All messages (`ALL`).
    All,
    /// Tracing messages and above (`TRACE`).
    Trace,
    /// Debugging messages and above (`DEBUG`).
    Debug,
    /// Informational messages and above (`INFO`).
    Info,
    /// Warnings and errors (`WARN`).
    Warn,
    /// Errors only (`ERROR`).
    Error,
    /// No messages (`OFF`).
    Off,
}

impl LogLevel {
    fn to_sys(self) -> sys::dxfg_v4_logging_level_t {
        match self {
            LogLevel::All => sys::DXFG_LOGGING_LEVEL_ALL,
            LogLevel::Trace => sys::DXFG_LOGGING_LEVEL_TRACE,
            LogLevel::Debug => sys::DXFG_LOGGING_LEVEL_DEBUG,
            LogLevel::Info => sys::DXFG_LOGGING_LEVEL_INFO,
            LogLevel::Warn => sys::DXFG_LOGGING_LEVEL_WARN,
            LogLevel::Error => sys::DXFG_LOGGING_LEVEL_ERROR,
            LogLevel::Off => sys::DXFG_LOGGING_LEVEL_OFF,
        }
    }

    /// The level of an SDK code; an unknown code is [`LogLevel::All`], the level of code 0.
    fn from_sys(level: sys::dxfg_v4_logging_level_t) -> LogLevel {
        match level {
            sys::DXFG_LOGGING_LEVEL_TRACE => LogLevel::Trace,
            sys::DXFG_LOGGING_LEVEL_DEBUG => LogLevel::Debug,
            sys::DXFG_LOGGING_LEVEL_INFO => LogLevel::Info,
            sys::DXFG_LOGGING_LEVEL_WARN => LogLevel::Warn,
            sys::DXFG_LOGGING_LEVEL_ERROR => LogLevel::Error,
            sys::DXFG_LOGGING_LEVEL_OFF => LogLevel::Off,
            _ => LogLevel::All,
        }
    }
}

/// A message of the log, as the SDK gives it to the handler of [`set_handler`]
/// (the arguments of `InterceptableLoggingListener.onLog`).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct LogRecord {
    /// The level of the message.
    pub level: LogLevel,
    /// The time of the message, in milliseconds since the Unix epoch.
    pub time: i64,
    /// The name of the Java thread that logged the message.
    pub thread_name: String,
    /// The identifier of the Java thread that logged the message.
    pub thread_id: i64,
    /// The name of the logger, if it has one.
    pub logger_name: Option<String>,
    /// The text of the message, without the time, the level and the thread.
    pub message: String,
    /// The exception logged with the message, if any.
    pub exception: Option<JavaException>,
    /// The whole line that the SDK would write: the message with the time, the level, the thread and the exception.
    pub formatted_message: String,
}

/// The handler of the messages.
type Handler = Arc<dyn Fn(&LogRecord) + Send + Sync>;

/// The handler that receives the messages; `None` until [`set_handler`] is called.
static HANDLER: Mutex<Option<Handler>> = Mutex::new(None);

/// The last level of [`set_log_level`]: the installation of the interceptor resets the level to `log.level`, so it is
/// set again afterwards.
static LEVEL: Mutex<Option<LogLevel>> = Mutex::new(None);

/// The interceptor installed in the SDK on the first [`set_handler`]; the SDK never removes it, so its handle is never
/// released and counts in [`live_handles`](crate::live_handles).
static LISTENER: OnceLock<Handle<sys::dxfg_v4_logging_listener_t>> = OnceLock::new();
static LISTENER_GUARD: Mutex<()> = Mutex::new(());

thread_local! {
    /// Set while the handler runs on this thread: a message the handler logs through the SDK is skipped.
    static IN_HANDLER: Cell<bool> = const { Cell::new(false) };
}

/// Routes the log of the SDK to `handler` (`InterceptableLogging.setListener`).
///
/// The first call installs an interceptor in the SDK: from then on the SDK writes no log to the console or to the
/// files, and gives every message of [`set_log_level`] and above to the handler. The SDK cannot remove the interceptor,
/// so a later call only replaces the handler; the messages logged meanwhile go to one of the two. If the first call
/// fails, the handler stays as it was and the next call tries again.
///
/// The handler runs on the thread of the SDK that logs the message, one call at a time: the other threads that log wait
/// for it, so keep it short. A panic of the handler goes to [`ErrorManager`](crate::ErrorManager), and a message that
/// the handler itself makes the SDK log is skipped. Like a listener, the handler must not wait for a thread that calls
/// the SDK, or both may wait forever.
#[doc(alias = "setListener")]
pub fn set_handler(handler: impl Fn(&LogRecord) + Send + Sync + 'static) -> Result<()> {
    // The old handler is dropped after the lock.
    let old = crate::lock(&HANDLER).replace(Arc::new(handler));
    let installed = crate::get_or_try_init(&LISTENER, &LISTENER_GUARD, || {
        let listener = Handle::create(|t, out| {
            // SAFETY: `log_trampoline` matches the function type and does not use the user data; the out-pointer is
            // valid for writes.
            unsafe { sys::dxfg_v4_logging_listener_new(t, Some(log_trampoline), std::ptr::null_mut(), out) }
        })?;
        // SAFETY: the listener is live; the SDK takes its own reference.
        call(|t| unsafe { sys::dxfg_v4_logging_set_listener(t, listener.as_ptr()) })?;
        let level = *crate::lock(&LEVEL);
        if let Some(level) = level {
            set_sdk_log_level(level)?;
        }
        Ok(listener)
    });
    match installed {
        Ok(_) => {
            drop(old);
            Ok(())
        }
        Err(error) => {
            let new = std::mem::replace(&mut *crate::lock(&HANDLER), old);
            drop(new);
            Err(error)
        }
    }
}

/// Sets the level of the messages that reach the handler of [`set_handler`], or, without a handler, the console or
/// the `log.file` file (`InterceptableLogging.setLogLevel`). The `log.level` system property sets the level at the
/// start (`INFO` by default); a level set before the first [`set_handler`] applies to the handler too. The SDK also
/// sets its root logger to the more verbose of this level and that of [`set_err_level`], so the messages below it are
/// not even created.
///
/// The SDK cannot give a handler the messages of the Java levels `FINER` and `CONFIG`, which classes of Java itself
/// log: the call that logs one returns [`Error::Java`](crate::Error::Java) with a `NullPointerException`, such as
/// [`DXEndpoint::connect`](crate::DXEndpoint::connect) at the level [`Trace`](LogLevel::Trace) or
/// [`All`](LogLevel::All). With a handler, use [`Debug`](LogLevel::Debug) or above; the console and the files take
/// every level.
#[doc(alias = "setLogLevel")]
pub fn set_log_level(level: LogLevel) -> Result<()> {
    // No lock is held while the SDK runs: a handler may set the level as well.
    set_sdk_log_level(level)?;
    *crate::lock(&LEVEL) = Some(level);
    Ok(())
}

fn set_sdk_log_level(level: LogLevel) -> Result<()> {
    // SAFETY: no pointers.
    call(|t| unsafe { sys::dxfg_v4_logging_set_log_level(t, level.to_sys()) })
}

/// Sets the level of the messages written to the `err.file` file, warnings and above by default
/// (`InterceptableLogging.setErrLevel`). It does nothing without that file or with a handler of [`set_handler`].
#[doc(alias = "setErrLevel")]
pub fn set_err_level(level: LogLevel) -> Result<()> {
    // SAFETY: no pointers.
    call(|t| unsafe { sys::dxfg_v4_logging_set_err_level(t, level.to_sys()) })
}

#[allow(clippy::too_many_arguments)] // The arguments of the SDK function type.
unsafe extern "C" fn log_trampoline(
    thread: *mut sys::graal_isolatethread_t,
    level: sys::dxfg_logging_level_t,
    time: i64,
    thread_name: *const c_char,
    thread_id: i64,
    logger_name: *const c_char,
    message: *const c_char,
    exception: *mut sys::dxfg_exception_t,
    formatted_message: *const c_char,
    _user_data: *mut c_void,
) {
    if isolate::exiting() || IN_HANDLER.get() {
        return;
    }
    let _ = panic::catch_unwind(AssertUnwindSafe(|| {
        let _scope = CallbackScope::enter(thread);
        let Some(handler) = crate::lock(&HANDLER).clone() else {
            return;
        };
        // SAFETY: the SDK passes NULL or NUL-terminated strings and a NULL or live exception, all valid for the call.
        let record = unsafe {
            LogRecord {
                level: LogLevel::from_sys(level),
                time,
                thread_name: isolate::string(thread_name).into_owned(),
                thread_id,
                logger_name: (!logger_name.is_null()).then(|| isolate::string(logger_name).into_owned()),
                message: isolate::string(message).into_owned(),
                exception: (!exception.is_null()).then(|| isolate::copy_exception(exception, 0)),
                formatted_message: isolate::string(formatted_message).into_owned(),
            }
        };
        IN_HANDLER.set(true);
        let result = panic::catch_unwind(AssertUnwindSafe(|| handler(&record)));
        IN_HANDLER.set(false);
        crate::error_manager::report_panic("logging", "the logging handler", result);
    }));
}
