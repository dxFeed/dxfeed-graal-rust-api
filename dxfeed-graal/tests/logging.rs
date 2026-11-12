//! The log of the SDK, the isolate arguments and the system properties. The handler of the log and the isolate are
//! global to a process, so each scenario runs in a child process ([`support::run_child`]).

mod support;

use std::net::TcpListener;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use dxfeed_graal::logging::{self, LogLevel, LogRecord};
use dxfeed_graal::{DXEndpoint, Error, ErrorManager, system};
use support::{is_child, run_child};

/// How long a scenario waits for a message of the log.
const WAIT: Duration = Duration::from_secs(10);

/// A local address that refuses connections, different from the ones returned before in this process.
fn refused_address() -> String {
    static USED: Mutex<Vec<u16>> = Mutex::new(Vec::new());
    loop {
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let mut used = USED.lock().unwrap();
        if !used.contains(&port) {
            used.push(port);
            return format!("127.0.0.1:{port}");
        }
    }
}

/// A new endpoint connecting to `address`, which makes the SDK log the attempts.
fn connect(address: &str) -> DXEndpoint {
    let endpoint = DXEndpoint::create().unwrap();
    endpoint.connect(address).unwrap();
    endpoint
}

/// A handler that sends the records to the returned receiver.
fn channel() -> (impl Fn(&LogRecord) + Send + Sync + 'static, Receiver<LogRecord>) {
    let (sender, receiver) = mpsc::channel();
    let sender = Mutex::new(sender);
    (
        move |record: &LogRecord| drop(sender.lock().unwrap().send(record.clone())),
        receiver,
    )
}

/// Receives records until one mentions `text`, for at most [`WAIT`]; returns all of them, that one last.
fn wait_for(receiver: &Receiver<LogRecord>, text: &str) -> Vec<LogRecord> {
    let deadline = Instant::now() + WAIT;
    let mut records = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let record = receiver
            .recv_timeout(left)
            .unwrap_or_else(|_| panic!("no record of {text}"));
        let found = record.formatted_message.contains(text);
        records.push(record);
        if found {
            return records;
        }
    }
}

/// Whether one of `records` mentions `text`.
fn mention(records: &[LogRecord], text: &str) -> bool {
    records.iter().any(|record| record.formatted_message.contains(text))
}

#[test]
fn the_log_goes_to_the_console_without_a_handler() {
    let (stdout, stderr) = run_child("child_the_log_goes_to_the_console_without_a_handler");
    assert!(format!("{stdout}{stderr}").contains("127.0.0.1:"), "{stdout}\n{stderr}");
}

#[test]
#[ignore = "run by the_log_goes_to_the_console_without_a_handler"]
fn child_the_log_goes_to_the_console_without_a_handler() {
    if !is_child() {
        return;
    }
    let endpoint = connect(&refused_address());
    thread::sleep(Duration::from_secs(2));
    endpoint.close().unwrap();
}

#[test]
fn handlers_receive_the_log_instead_of_the_console() {
    let (stdout, stderr) = run_child("child_handlers_receive_the_log_instead_of_the_console");
    assert!(
        !format!("{stdout}{stderr}").contains("127.0.0.1:"),
        "{stdout}\n{stderr}"
    );
}

#[test]
#[ignore = "run by handlers_receive_the_log_instead_of_the_console"]
fn child_handlers_receive_the_log_instead_of_the_console() {
    if !is_child() {
        return;
    }
    let (first, first_records) = channel();
    logging::set_handler(first).unwrap();
    let address = refused_address();
    let endpoint = connect(&address);
    let records = wait_for(&first_records, &address);
    endpoint.close().unwrap();
    let record = records.last().unwrap();
    assert!(record.level >= LogLevel::Info, "{record:?}");
    assert!(record.formatted_message.contains(&record.message), "{record:?}");
    assert!(!record.thread_name.is_empty() && record.time > 0, "{record:?}");

    let (second, second_records) = channel();
    logging::set_handler(second).unwrap();
    let address = refused_address();
    let endpoint = connect(&address);
    wait_for(&second_records, &address);
    endpoint.close().unwrap();
    let late: Vec<_> = first_records.try_iter().collect();
    assert!(!mention(&late, &address), "a replaced handler got {address}");

    logging::set_log_level(LogLevel::Off).unwrap();
    let silent = refused_address();
    let endpoint = connect(&silent);
    thread::sleep(Duration::from_secs(1));
    endpoint.close().unwrap();
    // `All` also passes the Java levels `FINER` and `CONFIG`, which `connect` logs.
    logging::set_log_level(LogLevel::All).unwrap();
    let address = refused_address();
    let endpoint = connect(&address);
    let records = wait_for(&second_records, &address);
    endpoint.close().unwrap();
    assert!(!mention(&records, &silent), "the level OFF passed {silent}");
    logging::set_err_level(LogLevel::Error).unwrap();

    let errors = Arc::new(Mutex::new(Vec::new()));
    let sink = errors.clone();
    ErrorManager::set_handler(move |error| sink.lock().unwrap().push(error.message.clone()));
    logging::set_handler(|_| panic!("boom")).unwrap();
    let endpoint = connect(&refused_address());
    let deadline = Instant::now() + WAIT;
    let reported = || {
        errors
            .lock()
            .unwrap()
            .iter()
            .any(|message| message == "the logging handler panicked: boom")
    };
    while !reported() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(50));
    }
    endpoint.close().unwrap();
    assert!(reported(), "{:?}", errors.lock().unwrap());
}

#[test]
fn a_level_set_before_the_handler_applies_to_it() {
    run_child("child_a_level_set_before_the_handler_applies_to_it");
}

#[test]
#[ignore = "run by a_level_set_before_the_handler_applies_to_it"]
fn child_a_level_set_before_the_handler_applies_to_it() {
    if !is_child() {
        return;
    }
    logging::set_log_level(LogLevel::Off).unwrap();
    let (handler, records) = channel();
    logging::set_handler(handler).unwrap();
    let silent = refused_address();
    let endpoint = connect(&silent);
    thread::sleep(Duration::from_secs(1));
    endpoint.close().unwrap();
    // `All` also passes the Java levels `FINER` and `CONFIG`, which `connect` logs.
    logging::set_log_level(LogLevel::All).unwrap();
    let address = refused_address();
    let endpoint = connect(&address);
    let seen = wait_for(&records, &address);
    endpoint.close().unwrap();
    assert!(!mention(&seen, &silent), "the level OFF passed {silent}");
}

#[test]
fn isolate_arguments_apply_before_the_isolate_only() {
    run_child("child_isolate_arguments_apply_before_the_isolate_only");
}

#[test]
#[ignore = "run by isolate_arguments_apply_before_the_isolate_only"]
fn child_isolate_arguments_apply_before_the_isolate_only() {
    if !is_child() {
        return;
    }
    assert!(matches!(
        system::set_isolate_arguments(["-Da=\0"]),
        Err(Error::InvalidArgument(_))
    ));
    system::set_isolate_arguments(["-Dignored=1"]).unwrap();
    system::set_isolate_arguments(["-Dfrom.arguments=42", "-Xmx256m"]).unwrap();
    assert_eq!(system::get_property("from.arguments").unwrap().as_deref(), Some("42"));
    assert_eq!(system::get_property("ignored").unwrap(), None);
    assert!(matches!(
        system::set_isolate_arguments(["-Dlate=1"]),
        Err(Error::IsolateCreated)
    ));

    system::set_properties([("first", "1"), ("second", "2")]).unwrap();
    assert_eq!(system::get_property("second").unwrap().as_deref(), Some("2"));
    assert!(matches!(
        system::set_properties([("third", "3"), ("fourth", "\0")]),
        Err(Error::InvalidArgument(_))
    ));
    assert_eq!(system::get_property("third").unwrap(), None);
}

#[test]
fn wrong_isolate_arguments_fail_every_call() {
    let (_, stderr) = run_child("child_wrong_isolate_arguments_fail_every_call");
    // In CI the SDK for macOS x86-64 printed nothing when it could not create the isolate; the others print the reason.
    if !cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        assert!(stderr.contains("64q"), "{stderr}");
    }
}

#[test]
#[ignore = "run by wrong_isolate_arguments_fail_every_call"]
fn child_wrong_isolate_arguments_fail_every_call() {
    if !is_child() {
        return;
    }
    system::set_isolate_arguments(["-Xmx64q"]).unwrap();
    assert!(matches!(DXEndpoint::create(), Err(Error::Graal(_))));
    assert!(matches!(system::get_property("a"), Err(Error::Graal(_))));
    assert!(matches!(
        system::set_isolate_arguments(["-Xmx64m"]),
        Err(Error::IsolateCreated)
    ));
}
