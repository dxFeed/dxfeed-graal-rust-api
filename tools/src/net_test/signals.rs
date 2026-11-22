//! SIGINT and SIGTERM (and SIGBREAK on Windows) as the NetTest specification wants them, as the C NetTest takes them:
//! the first signal asks the run to stop, the second ends the process at once with the default action of the signal.

use std::ffi::c_int;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use super::limiter::Stop;

/// `SIGBREAK` of the Windows C runtime (Ctrl+Break, and the closing of the console), which the libc crate lacks.
#[cfg(windows)]
const SIGBREAK: c_int = 21;

/// The signals the watcher takes.
#[cfg(windows)]
const WATCHED: [c_int; 3] = [libc::SIGINT, libc::SIGTERM, SIGBREAK];
#[cfg(not(windows))]
const WATCHED: [c_int; 2] = [libc::SIGINT, libc::SIGTERM];

/// What `signal` returns on a failure: `-1` as a handler (the libc crate types it as an `int` on Windows).
const SIG_ERR: libc::sighandler_t = !0;

/// How often the watcher thread looks at the flag of the handler.
const POLL_PERIOD: Duration = Duration::from_millis(20);

/// Whether a signal came; the handler may only touch a lock-free atomic.
static RECEIVED: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(signal: c_int) {
    if RECEIVED.swap(true, Ordering::SeqCst) {
        // The second signal: the tool may wait for something that does not check the stop (a large symbol file).
        // SAFETY: `signal` and `raise` may be called from a signal handler; the signal number is the one delivered.
        unsafe {
            libc::signal(signal, libc::SIG_DFL);
            libc::raise(signal);
        }
        return;
    }
    // Windows restores the default action before it calls the handler.
    // SAFETY: as above; the handler is a valid `extern "C" fn(c_int)`.
    unsafe {
        libc::signal(signal, handler());
    }
}

fn handler() -> libc::sighandler_t {
    on_signal as extern "C" fn(c_int) as libc::sighandler_t
}

/// The handler of the signals and a thread that turns the first signal into a stop request. Dropping it restores the
/// previous handlers.
pub struct SignalWatcher {
    done: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    previous: Vec<(c_int, libc::sighandler_t)>,
}

impl SignalWatcher {
    /// Installs the handler; the first signal sets `signalled` and requests the stop.
    pub fn install(stop: Arc<Stop>, signalled: Arc<AtomicBool>) -> Self {
        RECEIVED.store(false, Ordering::SeqCst);
        let previous = WATCHED
            .iter()
            // SAFETY: the signal numbers are valid and the handler only touches an atomic and calls `signal`/`raise`.
            .map(|&signal| (signal, unsafe { libc::signal(signal, handler()) }))
            .collect();
        let done = Arc::new(AtomicBool::new(false));
        let watching = Arc::clone(&done);
        let thread = std::thread::Builder::new()
            .name("NetTest signals".to_owned())
            .spawn(move || {
                while !watching.load(Ordering::SeqCst) {
                    if RECEIVED.load(Ordering::SeqCst) {
                        signalled.store(true, Ordering::SeqCst);
                        stop.request();
                        break;
                    }
                    std::thread::sleep(POLL_PERIOD);
                }
            })
            .ok();
        SignalWatcher { done, thread, previous }
    }

    /// Whether a signal came.
    pub fn received(&self) -> bool {
        RECEIVED.load(Ordering::SeqCst)
    }
}

impl Drop for SignalWatcher {
    fn drop(&mut self) {
        self.done.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        for &(signal, previous) in &self.previous {
            let restored = if previous == SIG_ERR { libc::SIG_DFL } else { previous };
            // SAFETY: the handler is the one `signal` returned for this signal, or the default action.
            unsafe {
                libc::signal(signal, restored);
            }
        }
    }
}
