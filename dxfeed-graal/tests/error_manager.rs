//! Errors without a caller: the handler and the journal of `ErrorManager`.

use std::net::TcpListener;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use dxfeed_graal::{DXEndpoint, Error, ErrorInfo, ErrorManager, Role, State};

/// `ErrorManager` is global: the tests of this file take turns and restore its defaults.
fn exclusive() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ErrorManager::reset_handler();
    ErrorManager::set_capacity(ErrorManager::DEFAULT_CAPACITY);
    ErrorManager::clear_errors();
    guard
}

/// A handler that collects the errors of `component`.
fn collect(component: &'static str) -> Arc<Mutex<Vec<ErrorInfo>>> {
    let errors = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&errors);
    ErrorManager::set_handler(move |error| {
        if error.component == component {
            sink.lock().unwrap().push(error.clone());
        }
    });
    errors
}

#[test]
fn a_report_goes_to_the_handler_and_the_journal() {
    let _guard = exclusive();
    let errors = collect("Test");
    ErrorManager::report("Test", "first\nline");
    let line = line!() - 1;
    ErrorManager::report("Test", "second");
    let errors = errors.lock().unwrap().clone();
    assert_eq!(errors.len(), 2);
    let first = &errors[0];
    assert_eq!(first.location, format!("error_manager.rs:{line}"));
    assert_eq!(first.message, "first\nline");
    assert_eq!(first.java_class_name, None);
    assert_eq!(first.thread_id, std::thread::current().id());
    assert!(first.time > 0);
    assert_eq!(errors[1].id, first.id + 1);
    assert_eq!(
        first.to_string(),
        format!("#{} Test (error_manager.rs:{line}): first line", first.id)
    );
    assert_eq!(ErrorManager::get_errors(), errors);
    assert_eq!(ErrorManager::get_last_error().as_ref(), errors.last());
    ErrorManager::reset_handler();
}

#[test]
fn a_java_exception_gives_its_class_and_stack_trace() {
    let _guard = exclusive();
    let endpoint = DXEndpoint::create_with_role(Role::Feed).unwrap();
    let error = endpoint.connect("not-a-scheme://[").unwrap_err();
    let Error::Java(java) = &error else {
        panic!("expected a Java exception, got {error:?}");
    };
    ErrorManager::set_handler(|_| {});
    ErrorManager::report_error("Test", &error);
    let info = ErrorManager::get_last_error().unwrap();
    assert_eq!(info.java_class_name.as_deref(), Some(java.get_class_name()));
    assert_eq!(info.java_stack_trace.as_deref(), Some(java.get_stack_trace()));
    assert_eq!(info.message, java.get_message());
    assert!(
        info.to_string().contains(&format!("{}: ", java.get_class_name())),
        "{info}"
    );

    ErrorManager::report_error("Test", &Error::Timeout);
    let info = ErrorManager::get_last_error().unwrap();
    assert_eq!(info.message, Error::Timeout.to_string());
    assert_eq!(info.java_class_name, None);
    ErrorManager::reset_handler();
}

#[test]
fn the_journal_keeps_the_last_errors() {
    let _guard = exclusive();
    ErrorManager::set_handler(|_| {});
    for i in 0..3 {
        ErrorManager::report("Test", i.to_string());
    }
    ErrorManager::set_capacity(2);
    assert_eq!(ErrorManager::get_capacity(), 2);
    let messages: Vec<_> = ErrorManager::get_errors().into_iter().map(|e| e.message).collect();
    assert_eq!(messages, ["1", "2"]);
    ErrorManager::report("Test", "3");
    let messages: Vec<_> = ErrorManager::get_errors().into_iter().map(|e| e.message).collect();
    assert_eq!(messages, ["2", "3"]);
    ErrorManager::clear_errors();
    assert_eq!(ErrorManager::get_last_error(), None);
    ErrorManager::set_capacity(0);
    ErrorManager::report("Test", "4");
    assert!(ErrorManager::get_errors().is_empty());
    ErrorManager::reset_handler();
}

#[test]
fn a_report_from_the_handler_goes_to_the_journal_only_and_a_panic_is_ignored() {
    let _guard = exclusive();
    let calls = Arc::new(Mutex::new(0));
    let counter = Arc::clone(&calls);
    ErrorManager::set_handler(move |error| {
        *counter.lock().unwrap() += 1;
        if error.message == "outer" {
            ErrorManager::report("Test", "inner");
            panic!("test panic in an error handler");
        }
    });
    ErrorManager::report("Test", "outer");
    assert_eq!(*calls.lock().unwrap(), 1);
    assert_eq!(ErrorManager::get_last_error().unwrap().message, "inner");
    // The handler still works after its panic.
    ErrorManager::report("Test", "next");
    assert_eq!(*calls.lock().unwrap(), 2);
    ErrorManager::reset_handler();
}

#[test]
fn a_panic_of_a_listener_is_reported() {
    let _guard = exclusive();
    let errors = collect("DXEndpoint");
    let endpoint = DXEndpoint::create().unwrap();
    let (sender, states) = mpsc::channel();
    endpoint
        .add_state_listener(move |_, new| {
            let _ = sender.send(new);
            panic!("test panic in a state listener");
        })
        .unwrap();
    // Nothing listens on the port, so the endpoint keeps connecting.
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    endpoint.connect(&format!("127.0.0.1:{port}")).unwrap();
    while states.recv_timeout(Duration::from_secs(10)).unwrap() != State::Connecting {}
    endpoint.close_and_await_termination().unwrap();
    let errors = errors.lock().unwrap().clone();
    assert!(!errors.is_empty());
    assert_eq!(
        errors[0].message,
        "an endpoint state listener panicked: test panic in a state listener"
    );
    assert!(errors[0].location.starts_with("listener.rs:"), "{}", errors[0]);
    ErrorManager::reset_handler();
}
