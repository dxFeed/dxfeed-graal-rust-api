//! The LatencyTest tool on a tape file written by a publisher; no network needed.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use dxfeed_graal::event::Quote;
use dxfeed_graal::{DXEndpoint, Event, Role};

/// A file in the temporary directory, removed when the test ends, whether it passes or not.
struct TempFile(PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Writes a text tape with the quotes of TEST1 on the exchange Q and of TEST2 on the exchange N.
fn write_tape(name: &str) -> TempFile {
    let file = TempFile(std::env::temp_dir().join(format!("dxfeed-graal-tools-{}-{name}", std::process::id())));
    let _ = std::fs::remove_file(&file.0);
    let events: Vec<Event> = (0..20)
        .map(|i| {
            let (symbol, exchange) = if i % 2 == 0 { ("TEST1", 'Q') } else { ("TEST2", 'N') };
            Quote {
                bid_time: 1_000_000,
                bid_exchange_code: exchange,
                bid_price: f64::from(i),
                ask_exchange_code: exchange,
                ..Quote::new(symbol)
            }
            .into()
        })
        .collect();
    // Without a subscription a publisher drops its events; the wildcard subscribes the tape to everything.
    let writer = DXEndpoint::new_builder()
        .with_role(Role::Publisher)
        .with_property("dxfeed.wildcard.enable", "true")
        .build()
        .unwrap();
    writer
        .connect(&format!("tape:{}[format=text]", file.0.display()))
        .unwrap();
    writer.get_publisher().publish_events(&events).unwrap();
    writer.await_processed().unwrap();
    writer.close_and_await_termination().unwrap();
    file
}

/// The number before the unit at the end of a table line, such as `2` of `  Sample size (N)          : 2 (events)`.
fn number(line: &str) -> usize {
    line.rsplit(':')
        .next()
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap()
}

#[test]
fn latency_test_skips_the_ignored_exchanges() {
    let file = write_tape("latency.txt");
    let address = format!("file:{}[speed=max,cycle]", file.0.display());
    let mut child = Command::new(env!("CARGO_BIN_EXE_Tools"))
        .args([
            "LatencyTest",
            &address,
            "Quote",
            "all",
            "--interval",
            "1",
            "--ignore-exchanges",
            "N",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (sender, received) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if line.starts_with("  ") && sender.send(line).is_err() {
                break;
            }
        }
    });
    // The cyclic tape never ends: read the table of an interval with samples, or until the time is up, then stop the
    // tool.
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut table = Vec::new();
    let mut measured = None;
    while measured.is_none()
        || !table
            .last()
            .is_some_and(|line: &String| line.starts_with("  Measurement interval"))
    {
        let left = deadline.saturating_duration_since(Instant::now());
        let Ok(line) = received.recv_timeout(left) else {
            break;
        };
        if line.starts_with("  Rate of events (avg)") {
            table.clear();
        }
        if line.starts_with("  Sample size (N)") && number(&line) > 0 {
            measured = table
                .iter()
                .find(|line: &&String| line.starts_with("  Rate of unique symbols"))
                .map(|line| number(line));
        }
        table.push(line);
    }
    child.kill().unwrap();
    child.wait().unwrap();
    // Only TEST1 comes: the quotes of TEST2 are on the ignored exchange.
    assert_eq!(measured, Some(1), "{table:?}");
    assert!(
        table.iter().any(|line| line == "  Measurement interval     : 1 (s)"),
        "{table:?}"
    );
}

#[test]
fn latency_test_rejects_a_long_exchange_code() {
    let output = Command::new(env!("CARGO_BIN_EXE_Tools"))
        .args(["LatencyTest", "file:nothing", "--ignore-exchanges", "Q,NY"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(
            "\nLatencyTest\n===========\n\n  \"ignoreExchanges\" argument parsing error. \"NY\" is not a \
             one-character exchange code.\n\nUsage:\n"
        ),
        "{stdout}"
    );
}
