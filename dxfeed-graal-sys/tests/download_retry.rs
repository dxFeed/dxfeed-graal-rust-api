//! Tests the retries of the SDK archive download in the build script.

#[path = "../build/retry.rs"]
mod retry;

use std::io::ErrorKind;
use std::time::Duration;

use retry::{ATTEMPTS, Failure, backoff, io_failure, retry, status_failure};

#[test]
fn only_429_and_5xx_statuses_are_transient() {
    for status in [429, 500, 502, 503, 504, 599] {
        assert_eq!(status_failure(status), Failure::Transient, "{status}");
    }
    for status in [400, 401, 403, 404, 408, 410, 451, 499, 600] {
        assert_eq!(status_failure(status), Failure::Permanent, "{status}");
    }
}

#[test]
fn network_io_errors_are_transient_and_local_ones_are_not() {
    for kind in [
        ErrorKind::TimedOut,
        ErrorKind::ConnectionRefused,
        ErrorKind::ConnectionReset,
        ErrorKind::ConnectionAborted,
        ErrorKind::NotConnected,
        ErrorKind::BrokenPipe,
        ErrorKind::UnexpectedEof,
        ErrorKind::Interrupted,
        ErrorKind::HostUnreachable,
        ErrorKind::NetworkUnreachable,
        ErrorKind::NetworkDown,
    ] {
        assert_eq!(io_failure(kind), Failure::Transient, "{kind:?}");
    }
    for kind in [
        ErrorKind::NotFound,
        ErrorKind::PermissionDenied,
        ErrorKind::InvalidData,
        ErrorKind::StorageFull,
        ErrorKind::Other,
    ] {
        assert_eq!(io_failure(kind), Failure::Permanent, "{kind:?}");
    }
}

#[test]
fn backoff_grows() {
    assert_eq!(backoff(1), Duration::from_secs(2));
    assert_eq!(backoff(2), Duration::from_secs(4));
    assert!(backoff(10) <= Duration::from_secs(32));
}

/// The result of one attempt: a digest or a classified error.
type Attempt = Result<&'static str, (Failure, &'static str)>;
/// The result of `retry`: a digest, or the last error and the number of attempts made.
type Outcome = Result<&'static str, (&'static str, u32)>;
/// A wait that `retry` asked for: the failed attempt, its error and the backoff.
type Wait = (u32, &'static str, Duration);

/// Runs `retry` over the given attempt results and returns its result and the waits it asked for.
fn run(results: Vec<Attempt>) -> (Outcome, Vec<Wait>) {
    let mut results = results.into_iter();
    let mut waits = Vec::new();
    let result = retry(
        ATTEMPTS,
        || results.next().expect("no more attempts expected"),
        |attempt, error, delay| waits.push((attempt, *error, delay)),
    );
    (result, waits)
}

#[test]
fn succeeds_at_once_without_waiting() {
    assert_eq!(run(vec![Ok("digest")]), (Ok("digest"), vec![]));
}

#[test]
fn retries_transient_failures_until_success() {
    let (result, waits) = run(vec![
        Err((Failure::Transient, "timeout")),
        Err((Failure::Transient, "502")),
        Ok("digest"),
    ]);
    assert_eq!(result, Ok("digest"));
    assert_eq!(waits, [(1, "timeout", backoff(1)), (2, "502", backoff(2))]);
}

#[test]
fn gives_up_after_all_attempts() {
    assert_eq!(ATTEMPTS, 3);
    let (result, waits) = run(vec![Err((Failure::Transient, "502")); 3]);
    assert_eq!(result, Err(("502", 3)));
    assert_eq!(waits.len(), 2);
}

#[test]
fn never_retries_a_permanent_failure() {
    assert_eq!(run(vec![Err((Failure::Permanent, "404"))]), (Err(("404", 1)), vec![]));
    let (result, waits) = run(vec![
        Err((Failure::Transient, "timeout")),
        Err((Failure::Permanent, "cannot write")),
    ]);
    assert_eq!(result, Err(("cannot write", 2)));
    assert_eq!(waits.len(), 1);
}
