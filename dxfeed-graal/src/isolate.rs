//! The process-wide isolate, thread attachment and the call helper.
//!
//! One isolate is created on first use and never torn down: tear-down waits for every Java thread and can hang. A
//! thread is attached on its first SDK call and detached by a thread-local guard when it exits, unless the process is
//! exiting. The isolate is created with the arguments of [`crate::system::set_isolate_arguments`], if any. Right after
//! its creation, it gets the system properties of the `dxfeed.system.properties` file and of the `DXFEED_*`
//! environment variables, as in the C++ and .NET APIs.

use std::borrow::Cow;
use std::cell::Cell;
use std::ffi::{CStr, CString, c_char, c_int};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use dxfeed_graal_sys as sys;

use crate::error::{Error, JavaException, Result};

/// The isolate thread of the calling thread, valid only on that thread.
pub(crate) type Thread = *mut sys::graal_isolatethread_t;

/// The isolate pointer; it is never torn down, so sharing it between threads is safe.
struct Isolate(*mut sys::graal_isolate_t);

// SAFETY: the pointer is only passed to graal functions, which are thread-safe for an isolate that lives forever.
unsafe impl Send for Isolate {}
// SAFETY: as above.
unsafe impl Sync for Isolate {}

/// The result of `graal_create_isolate`; a failure is kept and returned on every later call, never retried.
static ISOLATE: OnceLock<std::result::Result<Isolate, i32>> = OnceLock::new();

/// The isolate arguments set by [`crate::system::set_isolate_arguments`]; `None` once the isolate creation took them.
static ARGUMENTS: Mutex<Option<Vec<CString>>> = Mutex::new(Some(Vec::new()));

/// The version of `graal_create_isolate_params_t` that has the isolate arguments
/// (`__graal_create_isolate_params_version` of `graal_isolate.h`, which bindgen does not export).
const PARAMS_VERSION: c_int = 5;

/// The program name passed as the first isolate argument, which the isolate skips.
const PROGRAM: &CStr = c"dxfeed-graal";

/// Set by the `atexit` handler: from then on nothing calls the SDK from destructors.
static EXITING: AtomicBool = AtomicBool::new(false);

/// Set by [`crate::system::shutdown`]: from then on nothing new can be created.
static SHUT_DOWN: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    /// The crate has not attached this thread.
    Unattached,
    /// The crate attached this thread (`THREAD`) and detaches it when the thread exits.
    Owned,
    /// The guard has run: every later call attaches for its own duration only.
    Gone,
}

thread_local! {
    // No destructors, so these stay readable while other thread-locals are destroyed.
    static THREAD: Cell<Thread> = const { Cell::new(ptr::null_mut()) };
    static STATE: Cell<State> = const { Cell::new(State::Unattached) };
    static GUARD: Guard = const { Guard };
    // The isolate thread passed to the running listener callback, and how deeply callbacks are nested.
    static CALLBACK_THREAD: Cell<Thread> = const { Cell::new(ptr::null_mut()) };
    static CALLBACK_DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// Marks the calling thread as running a listener callback on `thread` until dropped.
pub(crate) struct CallbackScope {
    previous: Thread,
}

impl CallbackScope {
    pub(crate) fn enter(thread: Thread) -> Self {
        CALLBACK_DEPTH.set(CALLBACK_DEPTH.get() + 1);
        CallbackScope {
            previous: CALLBACK_THREAD.replace(thread),
        }
    }
}

impl Drop for CallbackScope {
    fn drop(&mut self) {
        CALLBACK_THREAD.set(self.previous);
        CALLBACK_DEPTH.set(CALLBACK_DEPTH.get() - 1);
    }
}

/// Whether the calling thread is running a listener callback.
pub(crate) fn in_callback() -> bool {
    CALLBACK_DEPTH.get() > 0
}

/// Detaches the thread the crate attached when the thread exits.
struct Guard;

impl Drop for Guard {
    fn drop(&mut self) {
        let thread = THREAD.replace(ptr::null_mut());
        if STATE.replace(State::Gone) == State::Owned {
            // SAFETY: `thread` was attached by this crate on this thread and is not used after this.
            unsafe { detach(thread) };
        }
    }
}

/// Whether the process is exiting (the `atexit` handler has run).
pub(crate) fn exiting() -> bool {
    EXITING.load(Ordering::Acquire)
}

/// Whether [`crate::system::shutdown`] has been called.
pub(crate) fn shut_down() -> bool {
    SHUT_DOWN.load(Ordering::Acquire)
}

pub(crate) fn set_shut_down() {
    SHUT_DOWN.store(true, Ordering::Release);
}

/// Sets the arguments of the isolate creation; [`Error::IsolateCreated`] once the creation has been attempted.
pub(crate) fn set_arguments(arguments: Vec<CString>) -> Result<()> {
    match crate::lock(&ARGUMENTS).as_mut() {
        Some(slot) => {
            *slot = arguments;
            Ok(())
        }
        None => Err(Error::IsolateCreated),
    }
}

/// Whether the isolate has been created (successfully or not).
pub(crate) fn isolate_created() -> bool {
    ISOLATE.get().is_some()
}

extern "C" fn on_exit() {
    // Atomics only: no allocation and no locks, as this may run under the Windows loader lock.
    EXITING.store(true, Ordering::Release);
}

unsafe extern "C" {
    fn atexit(callback: extern "C" fn()) -> c_int;
}

fn isolate() -> Result<*mut sys::graal_isolate_t> {
    let isolate = ISOLATE.get_or_init(|| {
        let mut isolate = ptr::null_mut();
        let mut thread = ptr::null_mut();
        // Leaked, as the isolate is created once per process: `graal_isolate.h` does not say whether the isolate
        // keeps `argv`.
        let arguments: &'static [CString] = Vec::leak(crate::lock(&ARGUMENTS).take().unwrap_or_default());
        let argv: &'static mut [*mut c_char] = Vec::leak(
            std::iter::once(PROGRAM)
                .chain(arguments.iter().map(CString::as_c_str))
                .map(|argument| argument.as_ptr().cast_mut())
                .collect(),
        );
        let mut params = sys::graal_create_isolate_params_t {
            version: PARAMS_VERSION,
            argc: c_int::try_from(argv.len()).unwrap_or(c_int::MAX),
            argv: argv.as_mut_ptr(),
            ..Default::default()
        };
        // Without arguments, null parameters select the defaults, as before the arguments existed.
        let params = if arguments.is_empty() {
            ptr::null_mut()
        } else {
            &raw mut params
        };
        // SAFETY: `params` is null or a zeroed structure of its version whose `argv` holds `argc` NUL-terminated
        // strings that live forever; the out-pointers are valid for writes.
        let code = unsafe { sys::graal_create_isolate(params, &mut isolate, &mut thread) };
        if code != 0 {
            return Err(code);
        }
        // SAFETY: `on_exit` is a plain function that lives as long as the process.
        unsafe { atexit(on_exit) };
        own(thread);
        load_system_properties(thread);
        Ok(Isolate(isolate))
    });
    match isolate {
        Ok(isolate) => Ok(isolate.0),
        Err(code) => Err(Error::Graal(*code)),
    }
}

/// The prefix of the environment variables that become system properties.
const ENV_PREFIX: &str = "DXFEED_";

/// The file of system properties read from the current directory, unless `DXFEED_dxfeed.system.properties` names
/// another one.
const SYSTEM_PROPERTIES_FILE: &str = "dxfeed.system.properties";

/// Sets as system properties the entries of the system properties file and then the `DXFEED_*` environment variables
/// without the prefix (`__` in a name becomes `:`), which override the file. A missing file, an entry that cannot be
/// set or a variable that is not Unicode is skipped, as in the C++ and .NET APIs.
fn load_system_properties(thread: Thread) {
    let file = std::env::var_os(format!("{ENV_PREFIX}{SYSTEM_PROPERTIES_FILE}"))
        .unwrap_or_else(|| SYSTEM_PROPERTIES_FILE.into());
    let mut properties = std::fs::read(file)
        .map(|bytes| parse_properties(String::from_utf8_lossy(&bytes).trim_start_matches('\u{feff}')))
        .unwrap_or_default();
    properties.extend(std::env::vars_os().filter_map(|(name, value)| {
        let key = name.to_str()?.replace("__", ":").strip_prefix(ENV_PREFIX)?.to_owned();
        Some((key, value.into_string().ok()?))
    }));
    for (key, value) in properties {
        let (Ok(key), Ok(value)) = (CString::new(key), CString::new(value)) else {
            continue;
        };
        // SAFETY: `thread` is the calling thread's isolate thread; both strings are NUL-terminated and outlive the call.
        let code = unsafe { sys::dxfg_v4_system_set_property(thread, key.as_ptr(), value.as_ptr()) };
        // A failure is skipped; `check` clears its exception.
        let _ = check(thread, code);
    }
}

/// The `key=value` entries of an INI text, in order, read as the C++ API reads them: `#` and `;` start comments, also
/// after a value; a key under a `[section]` line is named `section:key`; a repeated key keeps its first value; keys and
/// values are trimmed.
fn parse_properties(text: &str) -> Vec<(String, String)> {
    let mut section = String::new();
    let mut entries: Vec<(String, String)> = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with(['#', ';']) {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|line| line.strip_suffix(']')) {
            section = name.trim().to_owned();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = match (section.as_str(), key.trim()) {
            (_, "") => continue,
            ("", key) => key.to_owned(),
            (section, key) => format!("{section}:{key}"),
        };
        let value = value.split(['#', ';']).next().unwrap_or_default().trim();
        if !entries.iter().any(|(k, _)| *k == key) {
            entries.push((key, value.to_owned()));
        }
    }
    entries
}

/// Remembers `thread` as attached by the crate, or reports that the guard is no longer available.
fn own(thread: Thread) -> bool {
    if STATE.get() == State::Unattached && GUARD.try_with(|_| ()).is_ok() {
        THREAD.set(thread);
        STATE.set(State::Owned);
        true
    } else {
        false
    }
}

/// Runs `f` with the isolate thread of the calling thread, attaching it if needed.
pub(crate) fn with_thread<R>(f: impl FnOnce(Thread) -> R) -> Result<R> {
    let callback = CALLBACK_THREAD.get();
    if !callback.is_null() {
        return Ok(f(callback));
    }
    if STATE.get() == State::Owned {
        return Ok(f(THREAD.get()));
    }
    let isolate = isolate()?;
    // SAFETY: `isolate` is a live isolate.
    let current = unsafe { sys::graal_get_current_thread(isolate) };
    if !current.is_null() {
        // Attached by the creator or by someone else: use it, but neither cache nor detach it.
        return Ok(f(current));
    }
    let mut thread = ptr::null_mut();
    // SAFETY: `isolate` is a live isolate and the out-pointer is valid for writes.
    let code = unsafe { sys::graal_attach_thread(isolate, &mut thread) };
    if code != 0 {
        return Err(Error::Graal(code));
    }
    if own(thread) {
        return Ok(f(thread));
    }
    // The guard is gone (we run in another thread-local's destructor): attach for this call only.
    let result = f(thread);
    // SAFETY: `thread` was attached above on this thread and is not used after this.
    unsafe { detach(thread) };
    Ok(result)
}

/// Detaches `thread` unless the process is exiting.
///
/// # Safety
///
/// `thread` must have been attached by this crate on the calling thread and must not be used afterwards.
unsafe fn detach(thread: Thread) {
    if !exiting() {
        // SAFETY: guaranteed by the caller.
        unsafe { sys::graal_detach_thread(thread) };
    }
}

/// Runs an SDK function that returns `0` on success and `-1` with a thread exception on failure.
///
/// Every `-1` reads and clears the thread exception, so a stale exception never reaches a later failure.
pub(crate) fn call(f: impl FnOnce(Thread) -> i32) -> Result<()> {
    with_thread(|thread| check(thread, f(thread)))?
}

/// Maps the status of an SDK call made on `thread`.
pub(crate) fn check(thread: Thread, code: i32) -> Result<()> {
    match code {
        0 => Ok(()),
        -1 => Err(Error::Java(take_exception(thread))),
        code => Err(Error::Graal(code)),
    }
}

/// The most causes copied from one exception.
const MAX_CAUSES: usize = 32;

/// Takes the thread exception, copies it and releases it.
fn take_exception(thread: Thread) -> JavaException {
    let mut exception = ptr::null_mut();
    // SAFETY: `thread` is the calling thread's isolate thread and the out-pointer is valid for writes.
    let code = unsafe { sys::dxfg_v4_get_and_clear_thread_exception(thread, &mut exception) };
    if code != 0 || exception.is_null() {
        if code != 0 {
            // The conversion failed and stored its own exception: clear it so it does not reach a later call.
            let mut stale = ptr::null_mut();
            // SAFETY: as above; a NULL result is ignored by the release.
            unsafe {
                sys::dxfg_v4_get_and_clear_thread_exception(thread, &mut stale);
                sys::dxfg_v4_Exception_release(thread, stale);
            }
        }
        return JavaException::lost();
    }
    // SAFETY: `exception` was just written by the SDK and is released right after copying.
    let copy = unsafe { copy_exception(exception, 0) };
    // SAFETY: `exception` came from dxfg_v4_get_and_clear_thread_exception and is released once.
    unsafe { sys::dxfg_v4_Exception_release(thread, exception) };
    copy
}

/// Copies and releases an exception an SDK function returned, or returns `None` for NULL.
///
/// # Safety
///
/// `exception` must be NULL or a live SDK exception owned by the caller, not used afterwards.
pub(crate) unsafe fn take_returned_exception(exception: *mut sys::dxfg_v4_exception_t) -> Option<JavaException> {
    if exception.is_null() {
        return None;
    }
    // SAFETY: guaranteed by the caller.
    let copy = unsafe { copy_exception(exception, 0) };
    if !exiting() {
        // SAFETY: the exception is released once; a failed release leaks it.
        let _ = with_thread(|t| unsafe { sys::dxfg_v4_Exception_release(t, exception) });
    }
    Some(copy)
}

/// Copies an SDK exception and its causes, at most [`MAX_CAUSES`] deep.
///
/// # Safety
///
/// `exception` must point to a live SDK exception.
pub(crate) unsafe fn copy_exception(exception: *const sys::dxfg_v4_exception_t, depth: usize) -> JavaException {
    // SAFETY: the caller guarantees `exception` is live.
    let e = unsafe { &*exception };
    let cause = if e.cause.is_null() || depth + 1 >= MAX_CAUSES {
        None
    } else {
        // SAFETY: the cause belongs to the same live exception.
        Some(Box::new(unsafe { copy_exception(e.cause, depth + 1) }))
    };
    // SAFETY: the strings are NULL or NUL-terminated strings of the live exception.
    unsafe {
        JavaException::new(
            string(e.class_name).into_owned(),
            string(e.message).into_owned(),
            string(e.print_stack_trace).into_owned(),
            cause,
        )
    }
}

/// Reads an SDK string; NULL becomes empty, invalid UTF-8 is replaced.
///
/// # Safety
///
/// `s` must be NULL or a NUL-terminated string that outlives `'a`.
pub(crate) unsafe fn string<'a>(s: *const c_char) -> Cow<'a, str> {
    if s.is_null() {
        Cow::Borrowed("")
    } else {
        // SAFETY: the caller guarantees a NUL-terminated string.
        unsafe { CStr::from_ptr(s) }.to_string_lossy()
    }
}

/// Calls an SDK function that writes a new string to its out-pointer, copies the string and releases it; a NULL
/// string is an error. `what` names the caller in a report of a failed release.
pub(crate) fn out_string(what: &'static str, f: impl FnOnce(Thread, *mut *const c_char) -> i32) -> Result<String> {
    out_optional_string(what, f)?.ok_or_else(|| Error::Java(JavaException::null_result()))
}

/// As [`out_string`], with `None` for a NULL string.
pub(crate) fn out_optional_string(
    what: &'static str,
    f: impl FnOnce(Thread, *mut *const c_char) -> i32,
) -> Result<Option<String>> {
    let mut text = ptr::null();
    call(|t| f(t, &mut text))?;
    if text.is_null() {
        return Ok(None);
    }
    // SAFETY: the SDK wrote a NUL-terminated string that is released right after copying.
    let copy = unsafe { string(text) }.into_owned();
    // SAFETY: `text` came from the SDK to the caller and is released once.
    crate::error_manager::report_release(what, call(|t| unsafe { sys::dxfg_v4_String_release(t, text) }));
    Ok(Some(copy))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::set_property;

    #[test]
    fn properties_are_parsed_as_ini_entries() {
        let text = "# comment\n; comment\n dxfeed.address = demo.dxfeed.com:7300 # the demo feed\nempty=\n=no key\n\
                    no value\nurl=http://host/path?a=b ; note\ndxfeed.address=other:7300\n[ section ]\nkey=in section\n";
        let pairs: Vec<(&str, &str)> = vec![
            ("dxfeed.address", "demo.dxfeed.com:7300"),
            ("empty", ""),
            ("url", "http://host/path?a=b"),
            ("section:key", "in section"),
        ];
        let parsed = parse_properties(text);
        let parsed: Vec<(&str, &str)> = parsed.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        assert_eq!(parsed, pairs);
    }

    #[test]
    fn threads_share_one_isolate() {
        let threads: Vec<_> = (0..8)
            .map(|i| std::thread::spawn(move || set_property(&format!("isolate.test.parallel.{i}"), "1")))
            .collect();
        for thread in threads {
            thread.join().unwrap().unwrap();
        }
        let first = isolate().unwrap();
        assert_eq!(
            std::thread::spawn(|| isolate().unwrap() as usize).join().unwrap(),
            first as usize
        );
    }

    #[test]
    fn short_lived_threads_detach() {
        #[cfg(windows)]
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        #[cfg(windows)]
        let loader = {
            let stop = stop.clone();
            std::thread::spawn(move || {
                // Loader-lock contention: thread exits detach while another thread loads and frees a DLL.
                let name: Vec<u16> = "version.dll\0".encode_utf16().collect();
                while !stop.load(Ordering::Relaxed) {
                    // SAFETY: `name` is a NUL-terminated wide string; the module is freed right away.
                    unsafe {
                        let module = windows::LoadLibraryW(name.as_ptr());
                        if !module.is_null() {
                            windows::FreeLibrary(module);
                        }
                    }
                }
            })
        };
        for _ in 0..2000 {
            std::thread::spawn(|| set_property("isolate.test.churn", "1"))
                .join()
                .unwrap()
                .unwrap();
        }
        #[cfg(windows)]
        {
            stop.store(true, Ordering::Relaxed);
            loader.join().unwrap();
        }
        set_property("isolate.test.churn", "2").unwrap();
    }

    #[test]
    fn failure_does_not_leak_into_next_call() {
        // A NULL key makes the SDK throw.
        // SAFETY: the out-pointer is valid; a NULL key is rejected by the SDK with an exception.
        let failed = call(|t| unsafe { sys::dxfg_v4_system_get_property(t, ptr::null(), &mut ptr::null()) });
        let Err(Error::Java(e)) = failed else {
            panic!("expected a Java exception, got {failed:?}");
        };
        assert!(!e.get_class_name().is_empty());
        assert!(!e.get_stack_trace().is_empty());
        set_property("isolate.test.after-failure", "1").unwrap();
    }

    #[cfg(windows)]
    mod windows {
        use std::ffi::c_void;

        #[link(name = "kernel32")]
        unsafe extern "system" {
            pub fn LoadLibraryW(name: *const u16) -> *mut c_void;
            pub fn FreeLibrary(module: *mut c_void) -> i32;
        }
    }
}
