//! Retries of the SDK archive download. Free of network crates, so `tests/download_retry.rs` can test it.

use std::io::ErrorKind;
use std::time::Duration;

/// How many times the archive download is tried in all.
pub const ATTEMPTS: u32 = 3;

/// Whether a failed attempt may pass when it is tried again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// A timeout, a dropped connection or a server that is busy or failing.
    Transient,
    /// Anything that fails the same way again, such as a 404 or a file that cannot be written.
    Permanent,
}

/// An HTTP status that the server may not repeat: 429 and 5xx. Other 4xx statuses are permanent.
pub fn status_failure(status: u16) -> Failure {
    if status == 429 || (500..600).contains(&status) {
        Failure::Transient
    } else {
        Failure::Permanent
    }
}

/// A network I/O error that may pass on another connection.
pub fn io_failure(kind: ErrorKind) -> Failure {
    match kind {
        ErrorKind::TimedOut
        | ErrorKind::ConnectionRefused
        | ErrorKind::ConnectionReset
        | ErrorKind::ConnectionAborted
        | ErrorKind::NotConnected
        | ErrorKind::BrokenPipe
        | ErrorKind::UnexpectedEof
        | ErrorKind::Interrupted
        | ErrorKind::HostUnreachable
        | ErrorKind::NetworkUnreachable
        | ErrorKind::NetworkDown => Failure::Transient,
        _ => Failure::Permanent,
    }
}

/// The wait before the attempt that follows failed attempt `attempt` (1-based): 2 s, then 4 s.
pub fn backoff(attempt: u32) -> Duration {
    Duration::from_secs(2 << (attempt - 1).min(4))
}

/// Calls `attempt` up to `attempts` times while it fails transiently, calling `wait` with the failed attempt, its
/// error and the backoff before each retry. Returns the first success, or the last error and the number of attempts
/// made.
pub fn retry<T, E>(
    attempts: u32,
    mut attempt: impl FnMut() -> Result<T, (Failure, E)>,
    mut wait: impl FnMut(u32, &E, Duration),
) -> Result<T, (E, u32)> {
    let mut made = 1;
    loop {
        match attempt() {
            Ok(value) => return Ok(value),
            Err((Failure::Transient, error)) if made < attempts => {
                wait(made, &error, backoff(made));
                made += 1;
            }
            Err((_, error)) => return Err((error, made)),
        }
    }
}
