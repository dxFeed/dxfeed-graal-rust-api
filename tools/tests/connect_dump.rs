//! The Connect and Dump tools on tape files written by a publisher; no network needed.

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use dxfeed_graal::event::{Quote, Trade};
use dxfeed_graal::{DXEndpoint, Event, Role};

/// A file in the temporary directory, removed when the test ends, whether it passes or not.
struct TempFile(PathBuf);

impl TempFile {
    fn new(name: &str) -> TempFile {
        let path = std::env::temp_dir().join(format!("dxfeed-graal-tools-{}-{name}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        TempFile(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    /// The address that reads the file to its end at full speed.
    fn address(&self) -> String {
        format!("file:{}[speed=max]", self.0.display())
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Writes a text tape with two quotes and a trade.
fn write_tape(name: &str) -> TempFile {
    let file = TempFile::new(name);
    let events: Vec<Event> = vec![
        Quote {
            bid_price: 10.1,
            ask_price: 10.2,
            ..Quote::new("TEST1")
        }
        .into(),
        Quote {
            bid_price: 17.1,
            ask_price: 17.2,
            ..Quote::new("TEST2")
        }
        .into(),
        Trade {
            price: 10.15,
            ..Trade::new("TEST1")
        }
        .into(),
    ];
    // Without a subscription a publisher drops its events; the wildcard subscribes the tape to everything.
    let writer = DXEndpoint::new_builder()
        .with_role(Role::Publisher)
        .with_property("dxfeed.wildcard.enable", "true")
        .build()
        .unwrap();
    writer
        .connect(&format!("tape:{}[format=text]", file.path().display()))
        .unwrap();
    writer.get_publisher().publish_events(&events).unwrap();
    writer.await_processed().unwrap();
    writer.close_and_await_termination().unwrap();
    file
}

fn tools(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_Tools"))
        .args(args)
        .env("COLUMNS", "100")
        .output()
        .unwrap()
}

/// The events among the lines of the output, sorted: the lines of the tools may come with the log lines of the SDK.
fn events(output: &Output) -> Vec<String> {
    assert!(output.status.success(), "{output:?}");
    let mut events: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| line.starts_with("Quote{") || line.starts_with("Trade{"))
        .map(str::to_owned)
        .collect();
    events.sort();
    events
}

/// The beginning of a printed event: its type and symbol, such as `Quote{TEST1`.
fn head(event: &str) -> &str {
    &event[..event.find(',').unwrap()]
}

/// The beginnings of the printed events, in their order.
fn heads(events: &[String]) -> Vec<&str> {
    events.iter().map(|event| head(event)).collect()
}

#[test]
fn dump_prints_all_the_events_of_a_tape() {
    let file = write_tape("dump.txt");
    let address = file.address();
    let all = events(&tools(&["Dump", &address]));
    assert_eq!(heads(&all), ["Quote{TEST1", "Quote{TEST2", "Trade{TEST1"]);
    assert!(all[0].contains("bidPrice=10.1"), "{all:?}");

    let quotes = events(&tools(&["Dump", &address, "Quote"]));
    assert_eq!(heads(&quotes), ["Quote{TEST1", "Quote{TEST2"]);
    let test2 = events(&tools(&["Dump", &address, "Quote,Trade", "TEST2"]));
    assert_eq!(heads(&test2), ["Quote{TEST2"]);
    assert!(events(&tools(&["Dump", &address, "-q"])).is_empty());
}

#[test]
fn dump_tapes_the_events() {
    let file = write_tape("dump-in.txt");
    let tape = TempFile::new("dump-out.txt");
    let tape_address = format!("{}[format=text]", tape.path().display());
    let printed = events(&tools(&["Dump", &file.address(), "-q", "-t", &tape_address]));
    assert!(printed.is_empty());

    let replayed = events(&tools(&["Dump", &tape.address()]));
    assert_eq!(replayed, events(&tools(&["Dump", &file.address()])));
}

#[test]
fn dump_reports_unknown_types() {
    let output = tools(&["Dump", "file:nothing", "Quote,Nothing"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.contains("There are unknown event types: Nothing!\n List of available event types: "),
        "{error}"
    );
}

#[test]
fn connect_prints_the_events_of_a_tape() {
    let file = write_tape("connect.txt");
    let mut child = Command::new(env!("CARGO_BIN_EXE_Tools"))
        .args(["Connect", &file.address(), "Quote", "TEST1,TEST2"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (sender, received) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if line.starts_with("Quote{") && sender.send(line).is_err() {
                break;
            }
        }
    });
    // Connect runs until it is stopped: read until both symbols come or the time is up, then stop it.
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut seen = BTreeSet::new();
    while seen.len() < 2 {
        let left = deadline.saturating_duration_since(Instant::now());
        match received.recv_timeout(left) {
            Ok(line) => {
                seen.insert(head(&line).to_owned());
            }
            Err(_) => break,
        }
    }
    child.kill().unwrap();
    child.wait().unwrap();
    assert_eq!(
        seen.iter().map(String::as_str).collect::<Vec<_>>(),
        ["Quote{TEST1", "Quote{TEST2"]
    );
}

#[test]
fn a_closed_output_ends_connect() {
    let file = write_tape("connect-closed.txt");
    let mut child = Command::new(env!("CARGO_BIN_EXE_Tools"))
        .args(["Connect", &file.address(), "Quote", "TEST1,TEST2"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    // No one reads the output any more: the printing of the quotes fails and ends the process.
    drop(child.stdout.take());
    let deadline = Instant::now() + Duration::from_secs(30);
    while child.try_wait().unwrap().is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    let status = child.try_wait().unwrap();
    if status.is_none() {
        child.kill().unwrap();
        child.wait().unwrap();
    }
    assert!(status.is_some_and(|status| status.success()), "{status:?}");
}
