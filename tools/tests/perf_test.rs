//! The PerfTest tool on a tape file written by a publisher; no network needed.

use std::path::PathBuf;
use std::process::{Command, Output};

use dxfeed_graal::event::Quote;
use dxfeed_graal::{DXEndpoint, Event, Role};

/// A file in the temporary directory, removed when the test ends, whether it passes or not.
struct TempFile(PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Writes a text tape with quotes of two symbols.
fn write_tape(name: &str) -> TempFile {
    let file = TempFile(std::env::temp_dir().join(format!("dxfeed-graal-tools-{}-{name}", std::process::id())));
    let _ = std::fs::remove_file(&file.0);
    let events: Vec<Event> = (0..100)
        .map(|i| {
            Quote {
                bid_price: f64::from(i),
                ..Quote::new(if i % 2 == 0 { "TEST1" } else { "TEST2" })
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

fn tools(args: &[&str]) -> Output {
    let output = Command::new(env!("CARGO_BIN_EXE_Tools"))
        .args(args)
        .env("COLUMNS", "100")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    output
}

/// The lines of the output that are not log lines of the SDK.
fn lines(output: &Output) -> Vec<String> {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| !line.starts_with("I ") && !line.starts_with("W "))
        .map(str::to_owned)
        .collect()
}

#[test]
fn perf_test_counts_the_events_of_a_tape() {
    let file = write_tape("perf.txt");
    let address = format!("file:{}[speed=max]", file.0.display());
    let counted = lines(&tools(&[
        "PerfTest",
        &address,
        "Quote",
        "TEST1,TEST2",
        "--force-stream",
    ]));
    assert_eq!(
        counted.first().map(String::as_str),
        Some("Parsed symbols: 2"),
        "{counted:?}"
    );
    // Each event adds one plus its event time, which a text tape does not keep.
    let hash: u64 = counted.last().unwrap().parse().unwrap();
    assert_eq!(hash, 100, "{counted:?}");

    let detached = lines(&tools(&["PerfTest", &address, "Quote", "TEST1", "--detach-listener"]));
    assert_eq!(detached.first().map(String::as_str), Some("Parsed symbols: 1"));
    assert_eq!(detached.last().map(String::as_str), Some("0"));
}

#[test]
fn perf_test_writes_csv() {
    let file = write_tape("perf-csv.txt");
    let address = format!("file:{}[speed=max]", file.0.display());
    let csv = lines(&tools(&[
        "PerfTest",
        &address,
        "Quote",
        "TEST1",
        "-p",
        "PerfTest.format=csv",
    ]));
    assert!(
        csv[0].starts_with("Timestamp,Rate of events (avg) [events/s],"),
        "{csv:?}"
    );
    assert!(csv[0].ends_with(",Running time,Entity.Event"), "{csv:?}");
    assert_eq!(csv[1], "Parsed symbols: 1");
}

#[test]
fn perf_test_rejects_a_wrong_unsubscribe_time() {
    let output = Command::new(env!("CARGO_BIN_EXE_Tools"))
        .args([
            "PerfTest",
            "file:nothing",
            "Quote",
            "TEST1",
            "-p",
            "PerfTest.unsubscribeAfter=soon",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("Invalid PerfTest.unsubscribeAfter: soon"));
}
