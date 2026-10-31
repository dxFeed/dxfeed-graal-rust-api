//! Java system properties of the SDK and its shutdown.
//!
//! When the SDK starts, as in the C++ and .NET APIs, it sets the system properties of the `dxfeed.system.properties`
//! file in the current directory, or of the file named by the `DXFEED_dxfeed.system.properties` environment variable,
//! and then those of the `DXFEED_<property>` environment variables, which override the file. The file holds `key=value`
//! lines read as the C++ API reads them: `#` and `;` start comments, also after a value, a key under a `[section]` line
//! is named `section:key`, and a repeated key keeps its first value. A missing file is not an error.
//!
//! Most properties take effect only when set before the first endpoint is created.

use std::ptr;
use std::sync::Mutex;

use dxfeed_graal_sys as sys;

use crate::error::{Error, Result, c_string};
use crate::isolate::{self, call};
use crate::listener;

/// Sets the Java system property `key` to `value` (`System.setProperty`).
pub fn set_property(key: &str, value: &str) -> Result<()> {
    let (key, value) = (c_string(key, "property key")?, c_string(value, "property value")?);
    // SAFETY: both strings are NUL-terminated and outlive the call.
    call(|t| unsafe { sys::dxfg_v4_system_set_property(t, key.as_ptr(), value.as_ptr()) })
}

/// Returns the Java system property `key`, or `None` if it is not set (`System.getProperty`).
pub fn get_property(key: &str) -> Result<Option<String>> {
    let key = c_string(key, "property key")?;
    let mut value = ptr::null();
    // SAFETY: the key is NUL-terminated and the out-pointer is valid for writes.
    call(|t| unsafe { sys::dxfg_v4_system_get_property(t, key.as_ptr(), &mut value) })?;
    if value.is_null() {
        return Ok(None);
    }
    // SAFETY: the SDK wrote a NUL-terminated string that is released right after copying.
    let copy = unsafe { isolate::string(value) }.into_owned();
    // SAFETY: `value` came from dxfg_v4_system_get_property and is released once.
    crate::error_manager::report_release(
        "System",
        call(|t| unsafe { sys::dxfg_v4_system_release_property(t, value) }),
    );
    Ok(Some(copy))
}

/// Shuts the SDK down (`dxfg_v4_system_shutdown`): stops the calls of all listeners, waiting for the running ones,
/// then closes all endpoints and waits for their termination. It cannot be undone.
///
/// It is an advanced function with no counterpart in the other dxFeed APIs, for an orderly stop of everything at once.
/// An application usually closes its endpoints with
/// [`DXEndpoint::close_and_await_termination`](crate::DXEndpoint::close_and_await_termination) instead.
///
/// Afterwards every call that creates something (an endpoint, a feed, a subscription, a listener, an executor) returns
/// [`Error::ShutDown`]; existing values keep working, so dropping and closing them is safe. Calling it again, also from
/// several threads at once, waits for the first call and returns `Ok`.
///
/// It is not needed before the process exits: the exit does not wait for the SDK threads. Call it from a normal
/// context of the application: inside a listener it returns [`Error::InCallback`], and holding a lock that a listener
/// takes while calling it deadlocks. The endpoints' tasks are waited for as well, so call
/// [`ManualExecutor::process_all_pending`](crate::ManualExecutor::process_all_pending) from another thread if an
/// endpoint uses a [`ManualExecutor`](crate::ManualExecutor).
pub fn shutdown() -> Result<()> {
    static DONE: Mutex<bool> = Mutex::new(false);
    if isolate::in_callback() {
        return Err(Error::InCallback);
    }
    let mut done = crate::lock(&DONE);
    if *done {
        return Ok(());
    }
    isolate::set_shut_down();
    if isolate::isolate_created() {
        // SAFETY: no arguments besides the isolate thread.
        call(|t| unsafe { sys::dxfg_v4_system_shutdown(t) })?;
        // Listeners removed inside callbacks are released before this returns.
        listener::drain_release_thread();
    }
    *done = true;
    Ok(())
}
