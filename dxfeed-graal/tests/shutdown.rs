//! Shutdown and process exit. Each scenario runs in a child process, because shutdown and exit are
//! process-wide: the parent test re-runs this executable with `<scenario> --exact --ignored` and fails if the child
//! fails or does not exit within 60 seconds. Scenarios that test exit with live handles leave through
//! `process::exit(0)`, so no `Drop` runs and the `atexit` path is what executes.

mod support;

use std::io::Read;
use std::net::TcpListener;
use std::process::{self, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use dxfeed_graal::event::Quote;
use dxfeed_graal::{DXEndpoint, DXFeedSubscription, Error, ManualExecutor, Role, live_handles, system};
use support::{Hub, collect, is_time, quote, quote_with_bid};

/// Set for the child processes; the scenarios do nothing without it.
const CHILD: &str = "DXFEED_GRAAL_TEST_CHILD";

/// Runs `scenario` in a child process and returns its stderr; panics if it fails or does not exit within 60 seconds.
fn run(scenario: &str) -> String {
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([scenario, "--exact", "--ignored", "--nocapture"])
        .env(CHILD, "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let read = |mut pipe: Box<dyn Read + Send>| {
        thread::spawn(move || {
            let mut text = String::new();
            let _ = pipe.read_to_string(&mut text);
            text
        })
    };
    let stdout = read(Box::new(child.stdout.take().unwrap()));
    let stderr = read(Box::new(child.stderr.take().unwrap()));
    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("{scenario} did not exit within 60 seconds");
        }
        thread::sleep(Duration::from_millis(50));
    };
    let (stdout, stderr) = (stdout.join().unwrap(), stderr.join().unwrap());
    assert!(status.success(), "{scenario} failed with {status}\n{stdout}\n{stderr}");
    // A scenario name that matches no test makes libtest run nothing and exit with 0. The header is printed before the
    // test starts, so it is also there when the scenario leaves through `process::exit`.
    assert!(
        stdout.contains("running 1 test\n"),
        "{scenario} did not run exactly one test\n{stdout}\n{stderr}"
    );
    stderr
}

/// Whether this process is a child that runs a scenario.
fn is_child() -> bool {
    std::env::var_os(CHILD).is_some()
}

/// A feed endpoint connected to a local socket, with a subscription and a listener.
fn connected() -> (TcpListener, DXEndpoint, DXFeedSubscription<Quote>) {
    let peer = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = DXEndpoint::create_with_role(Role::Feed).unwrap();
    endpoint
        .connect(&format!("127.0.0.1:{}", peer.local_addr().unwrap().port()))
        .unwrap();
    let subscription = endpoint.get_feed().create_subscription::<Quote>().unwrap();
    subscription.add_event_listener(|_| {}).unwrap();
    subscription.add_symbols(&["AAPL"]).unwrap();
    // Let the endpoint connect, so its connection threads run.
    let _ = peer.accept();
    thread::sleep(Duration::from_millis(200));
    (peer, endpoint, subscription)
}

#[test]
fn shutdown_stops_delivery_and_creation() {
    let stderr = run("child_shutdown_stops_delivery_and_creation");
    assert!(!stderr.contains("dxfeed-graal:"), "{stderr}");
}

#[test]
#[ignore = "run by shutdown_stops_delivery_and_creation"]
fn child_shutdown_stops_delivery_and_creation() {
    if !is_child() {
        return;
    }
    let hub = Hub::new();
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    let events = collect(&subscription);
    subscription.add_symbols(&["AAPL"]).unwrap();
    hub.publish_and_process(&[quote("AAPL").into()]);
    assert!(events.try_recv().is_ok());

    // Shutdown waits for the endpoint's tasks: a manual executor needs another thread to run them meanwhile.
    let stop = Arc::new(AtomicBool::new(false));
    let pump = {
        let (executor, stop) = (hub.executor.clone(), stop.clone());
        thread::spawn(move || {
            while !stop.load(Ordering::Acquire) {
                let _ = executor.process_all_pending();
                thread::sleep(Duration::from_millis(10));
            }
        })
    };
    system::shutdown().unwrap();
    stop.store(true, Ordering::Release);
    pump.join().unwrap();
    system::shutdown().unwrap();
    let _ = hub.publisher.publish_events(&[quote_with_bid("AAPL", 2.0).into()]);
    let _ = hub.executor.process_all_pending();
    assert!(events.try_recv().is_err());
    assert!(matches!(
        DXEndpoint::create_with_role(Role::LocalHub),
        Err(Error::ShutDown)
    ));
    assert!(matches!(ManualExecutor::new(), Err(Error::ShutDown)));
    assert!(matches!(subscription.add_event_listener(|_| {}), Err(Error::ShutDown)));

    subscription.close().unwrap();
    hub.endpoint.close().unwrap();
    drop(subscription);
    drop(hub);
    assert_eq!(live_handles(), 0);
}

#[test]
fn shutdown_in_listener_is_refused() {
    run("child_shutdown_in_listener_is_refused");
}

#[test]
#[ignore = "run by shutdown_in_listener_is_refused"]
fn child_shutdown_in_listener_is_refused() {
    if !is_child() {
        return;
    }
    let hub = Hub::new();
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    let (sender, results) = mpsc::channel();
    subscription
        .add_event_listener(move |_| {
            let _ = sender.send(system::shutdown());
        })
        .unwrap();
    subscription.add_symbols(&["AAPL"]).unwrap();
    hub.publish_and_process(&[quote("AAPL").into()]);
    assert!(matches!(results.try_recv(), Ok(Err(Error::InCallback))));
}

#[test]
fn display_after_shutdown() {
    let stderr = run("child_display_after_shutdown");
    let event_time = stderr
        .lines()
        .find_map(|line| line.strip_prefix("eventTime="))
        .unwrap_or_else(|| panic!("no time printed\n{stderr}"));
    // The SDK formats the time, or the time prints as its milliseconds if the SDK refuses.
    let formatted = is_time(event_time) && event_time.ends_with(".123");
    assert!(formatted || event_time == "1700000000123", "{event_time}");
}

#[test]
#[ignore = "run by display_after_shutdown"]
fn child_display_after_shutdown() {
    if !is_child() {
        return;
    }
    system::shutdown().unwrap();
    let quote = Quote {
        event_time: 1_700_000_000_123,
        ..Quote::new("AAPL")
    };
    let text = quote.to_string();
    let event_time = text.split(", ").find_map(|f| f.strip_prefix("eventTime=")).unwrap();
    eprintln!("eventTime={event_time}");
}

#[test]
fn concurrent_shutdown() {
    run("child_concurrent_shutdown");
}

#[test]
#[ignore = "run by concurrent_shutdown"]
fn child_concurrent_shutdown() {
    if !is_child() {
        return;
    }
    let (_peer, _endpoint, _subscription) = connected();
    let threads: Vec<_> = (0..2).map(|_| thread::spawn(system::shutdown)).collect();
    for thread in threads {
        thread.join().unwrap().unwrap();
    }
}

#[test]
fn exit_during_callback() {
    run("child_exit_during_callback");
}

#[test]
#[ignore = "run by exit_during_callback"]
fn child_exit_during_callback() {
    if !is_child() {
        return;
    }
    let hub = Hub::with_default_executor();
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    let (started, wait_started) = mpsc::channel();
    let (_release, blocked) = mpsc::channel::<()>();
    subscription
        .add_event_listener(move |_| {
            let _ = started.send(());
            let _ = blocked.recv();
        })
        .unwrap();
    subscription.add_symbols(&["AAPL"]).unwrap();
    hub.publisher.publish_events(&[quote("AAPL").into()]).unwrap();
    wait_started.recv_timeout(Duration::from_secs(10)).unwrap();
    process::exit(0);
}

#[test]
fn exit_connected_without_shutdown() {
    run("child_exit_connected_without_shutdown");
}

#[test]
#[ignore = "run by exit_connected_without_shutdown"]
fn child_exit_connected_without_shutdown() {
    if !is_child() {
        return;
    }
    let (_peer, _endpoint, _subscription) = connected();
    process::exit(0);
}

#[test]
fn return_connected_after_drop() {
    run("child_return_connected_after_drop");
}

#[test]
#[ignore = "run by return_connected_after_drop"]
fn child_return_connected_after_drop() {
    if !is_child() {
        return;
    }
    let (peer, endpoint, subscription) = connected();
    drop(subscription);
    drop(endpoint);
    drop(peer);
}

#[test]
fn return_connected_after_shutdown() {
    run("child_return_connected_after_shutdown");
}

#[test]
#[ignore = "run by return_connected_after_shutdown"]
fn child_return_connected_after_shutdown() {
    if !is_child() {
        return;
    }
    let (_peer, _endpoint, _subscription) = connected();
    system::shutdown().unwrap();
}

#[test]
fn return_with_thread_in_call_loop() {
    run("child_return_with_thread_in_call_loop");
}

#[test]
#[ignore = "run by return_with_thread_in_call_loop"]
fn child_return_with_thread_in_call_loop() {
    if !is_child() {
        return;
    }
    let (started, wait_started) = mpsc::channel();
    thread::spawn(move || {
        let _ = started.send(());
        loop {
            let _ = system::get_property("user.dir");
        }
    });
    wait_started.recv_timeout(Duration::from_secs(10)).unwrap();
    thread::sleep(Duration::from_millis(100));
}
