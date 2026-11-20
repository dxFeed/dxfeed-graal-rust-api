//! The NetTest tool as a child process; the network is loopback only.

use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex, Once};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use dxfeed_graal::event::{TimeAndSale, Trade};
use dxfeed_graal::{DXEndpoint, Role, system};
use regex::Regex;

/// The summary line of the C tool.
const SUMMARY_LINE_REGEX: &str =
    "[{][*]NetTestProducerSide-[^*]+[*][}] data [0-9,]+ rps, lag [0-9,]+ us, instances [0-9]+, connections [0-9]+";
/// The runtime line of the C tool.
const RUNTIME_LINE_REGEX: &str = "[{][*]NetTestRuntime-[^*]+[*][}] heap ([0-9,]+ MB|n/a) of ([0-9,]+ MB|n/a), gc \
                                  ([0-9,]+ [(][0-9,]+ ms[)]|n/a), cpu ([0-9,]+%|n/a)";
/// The longest run of a test before the child is killed.
const DEADLINE: Duration = Duration::from_secs(60);
/// The clock skew allowed between the stamp of an event and the test's clock.
const ALLOWED_SKEW_MILLIS: i64 = 2000;

/// A file in the temporary directory, removed when the test ends, whether it passes or not.
struct TempFile(PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// The five symbols of the IPF fixture, not in alphabetical order.
const FIXTURE_SYMBOLS: [&str; 5] = ["IBM", "AAPL", "MSFT", "GOOG", "TSLA"];

fn write_fixture(name: &str) -> TempFile {
    let file = TempFile(std::env::temp_dir().join(format!("dxfeed-graal-tools-{}-{name}", std::process::id())));
    let mut text = "#STOCK::=TYPE,SYMBOL,DESCRIPTION\n".to_owned();
    for symbol in FIXTURE_SYMBOLS {
        text.push_str(&format!("STOCK,{symbol},{symbol} Inc\n"));
    }
    text.push_str("##COMPLETE\n");
    std::fs::write(&file.0, text).unwrap();
    file
}

/// What a finished child printed.
#[derive(Debug)]
struct Outcome {
    status: ExitStatus,
    stdout: String,
    stderr: String,
}

impl Outcome {
    fn code(&self) -> Option<i32> {
        self.status.code()
    }

    fn summaries(&self) -> Vec<&str> {
        self.stdout
            .lines()
            .filter(|line| line.starts_with("{*NetTestProducerSide-"))
            .collect()
    }
}

/// A running `Tools NetTest`, with its output read on threads so that a full pipe never blocks it.
struct Tool {
    child: Child,
    stdout: JoinHandle<String>,
    stderr: JoinHandle<String>,
}

fn read_all(mut pipe: impl Read + Send + 'static) -> JoinHandle<String> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = pipe.read_to_end(&mut bytes);
        String::from_utf8_lossy(&bytes).into_owned()
    })
}

impl Tool {
    fn start(args: &[&str]) -> Tool {
        let mut child = Command::new(env!("CARGO_BIN_EXE_Tools"))
            .arg("NetTest")
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = read_all(child.stdout.take().unwrap());
        let stderr = read_all(child.stderr.take().unwrap());
        Tool { child, stdout, stderr }
    }

    /// Waits for the exit; kills the child and fails the test after the deadline.
    fn finish(mut self) -> Outcome {
        let deadline = Instant::now() + DEADLINE;
        let status = loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                break Some(status);
            }
            if Instant::now() >= deadline {
                let _ = self.child.kill();
                let _ = self.child.wait();
                break None;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let stdout = self.stdout.join().unwrap();
        let stderr = self.stderr.join().unwrap();
        let Some(status) = status else {
            panic!("NetTest did not end in {DEADLINE:?}\nstdout:\n{stdout}\nstderr:\n{stderr}");
        };
        Outcome { status, stdout, stderr }
    }
}

fn run(args: &[&str]) -> Outcome {
    Tool::start(args).finish()
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

/// The milliseconds since the Unix epoch of a packed time and sequence.
fn time_millis(packed: i64) -> i64 {
    (packed >> 32) * 1000 + ((packed >> 22) & 0x3FF)
}

#[test]
fn a_client_address_without_a_listener_runs_and_prints_the_statistics() {
    let outcome = run(&[
        "p",
        "127.0.0.1:1",
        "-S",
        "100",
        "-r",
        "*/5k",
        "-s",
        "1s",
        "--duration",
        "3s",
    ]);
    assert_eq!(outcome.code(), Some(0), "{outcome:#?}");
    assert!(
        Regex::new(SUMMARY_LINE_REGEX).unwrap().is_match(&outcome.stdout),
        "{outcome:#?}"
    );
    assert!(outcome.stdout.contains("instances 1, connections 0"), "{outcome:#?}");
    assert!(
        Regex::new("data [1-9][0-9,]* rps").unwrap().is_match(&outcome.stdout),
        "{outcome:#?}"
    );
    assert!(outcome.stdout.contains("\n    NetTest.1 data "), "{outcome:#?}");
    assert!(
        Regex::new(RUNTIME_LINE_REGEX).unwrap().is_match(&outcome.stdout),
        "{outcome:#?}"
    );
    assert!(
        Regex::new("NetTestRuntime-NetTest[*][}] heap [0-9,]+ MB of [0-9,]+ MB")
            .unwrap()
            .is_match(&outcome.stdout),
        "{outcome:#?}"
    );
    // Each block is a summary line, the instance lines and the runtime line.
    let lines: Vec<&str> = outcome.stdout.lines().collect();
    assert!(!lines.is_empty() && lines.len().is_multiple_of(3), "{outcome:#?}");
    for block in lines.chunks(3) {
        assert!(
            block[0].starts_with("{*NetTestProducerSide-NetTest*} data "),
            "{block:?}"
        );
        assert!(block[1].starts_with("    NetTest.1 data "), "{block:?}");
        assert!(block[2].starts_with("{*NetTestRuntime-NetTest*} heap "), "{block:?}");
    }
}

#[test]
fn an_unknown_flag_is_a_bad_command_line() {
    let outcome = run(&["p", "127.0.0.1:1", "--no-such-flag"]);
    assert_eq!(outcome.code(), Some(2), "{outcome:#?}");
    assert!(
        outcome
            .stderr
            .starts_with("NetTest: unknown flag --no-such-flag\n\nUsage: Tools NetTest p <address> -S <symbols>"),
        "{outcome:#?}"
    );
    assert!(outcome.stdout.is_empty(), "{outcome:#?}");

    let legacy = run(&["p", "127.0.0.1:1", "-S", "1", "--legacy"]);
    assert_eq!(legacy.code(), Some(2), "{legacy:#?}");
    assert!(legacy.stderr.contains("--legacy"), "{legacy:#?}");
}

#[test]
fn no_arguments_print_the_usage() {
    let outcome = run(&[]);
    assert_eq!(outcome.code(), Some(2), "{outcome:#?}");
    assert!(
        outcome
            .stdout
            .starts_with("Usage: Tools NetTest p <address> -S <symbols> [options]\n\n"),
        "{outcome:#?}"
    );
}

#[test]
fn an_option_that_the_isolate_rejects_is_a_bad_command_line() {
    let outcome = run(&[
        "p",
        "127.0.0.1:1",
        "-Xmx64m",
        "-S",
        "100",
        "--duration",
        "1s",
        "-XX:+NetTestNoSuchOption",
        "-Xmx64m",
    ]);
    assert_eq!(outcome.code(), Some(2), "{outcome:#?}");
    assert_eq!(outcome.stdout, "", "{outcome:#?}");
    // The last line names the isolate options in command-line order, repeats kept; the reason that GraalVM prints
    // itself stays above it.
    // The regex of the spec with \S spelled in ASCII: the regex crate of the tests is built without Unicode classes.
    let pattern =
        Regex::new(r"^NetTest: the isolate rejected one of the options: ([^ \t\r\n]+(?: [^ \t\r\n]+)*)$").unwrap();
    let last = outcome.stderr.lines().last().unwrap_or_default();
    let options = pattern.captures(last).map(|captures| captures[1].to_owned());
    assert_eq!(
        options.as_deref(),
        Some("-Xmx64m -XX:+NetTestNoSuchOption -Xmx64m"),
        "{outcome:#?}"
    );
    assert!(outcome.stderr.ends_with('\n'), "{outcome:#?}");
}

#[test]
fn the_gc_options_reach_the_isolate() {
    // A young generation of 2 MB fills several times in 3 seconds of copied batches.
    let outcome = run(&[
        "p",
        "127.0.0.1:1",
        "-S",
        "100",
        "-r",
        "*/5k",
        "-s",
        "1s",
        "--duration",
        "3s",
        "-Xmx64m",
        "-Xmn2m",
        "-Xss1m",
        "-XX:+PrintGC",
    ]);
    assert_eq!(outcome.code(), Some(0), "{outcome:#?}");
    assert!(
        Regex::new(r"GC\([0-9]+\) Pause").unwrap().is_match(&outcome.stderr),
        "{outcome:#?}"
    );
}

#[test]
fn a_taken_server_port_fails_the_start() {
    let (taken, port) = take_wildcard_port();
    let address = format!(":{port}");
    let outcome = run(&["p", &address, "-S", "10", "--duration", "10s"]);
    drop(taken);
    assert_eq!(outcome.code(), Some(1), "{outcome:#?}");
    assert!(
        outcome.stderr.contains(&format!(
            "NetTest: the server socket ':{port}' (NetTest.1) did not start in 3000 ms (is the port already in use?)\n"
        )),
        "{outcome:#?}"
    );
}

/// Takes a free port on both wildcard addresses, as a server socket of the SDK binds it: the IPv6 socket of the
/// connector is dual-stack, and on macOS it may share the port with a socket that holds only the IPv4 address.
fn take_wildcard_port() -> (Vec<TcpListener>, u16) {
    let mut taken = Vec::new();
    let ipv6_port = match TcpListener::bind("[::]:0") {
        Ok(listener) => {
            let port = listener.local_addr().unwrap().port();
            taken.push(listener);
            port
        }
        Err(_) => 0,
    };
    // On Linux the dual-stack IPv6 socket already holds the IPv4 address, so this bind fails there.
    if let Ok(listener) = TcpListener::bind(("0.0.0.0", ipv6_port)) {
        taken.push(listener);
    }
    let port = taken
        .first()
        .expect("no wildcard address to bind")
        .local_addr()
        .unwrap()
        .port();
    (taken, port)
}

/// The events that the feed of the test received.
#[derive(Debug, Default)]
struct Collected {
    events: usize,
    foreign_symbols: usize,
    stale_time: usize,
    bad_nano_part: usize,
    with_nano_part: usize,
}

impl Collected {
    fn add(&mut self, symbol: &str, packed: i64, nano_part: i32, start_millis: i64) {
        self.events += 1;
        if !FIXTURE_SYMBOLS.contains(&symbol) {
            self.foreign_symbols += 1;
        }
        let millis = time_millis(packed);
        if millis < start_millis - ALLOWED_SKEW_MILLIS || millis > now_millis() + ALLOWED_SKEW_MILLIS {
            self.stale_time += 1;
        }
        if !(0..1_000_000).contains(&nano_part) {
            self.bad_nano_part += 1;
        } else if nano_part != 0 {
            self.with_nano_part += 1;
        }
    }
}

/// The nanosecond part of the time goes on the wire only when the property is set before the first endpoint of the
/// test process.
fn nano_time() {
    static SET: Once = Once::new();
    SET.call_once(|| system::set_property(DXEndpoint::DXSCHEME_NANO_TIME_PROPERTY, "true").unwrap());
}

/// Runs the tool on a free loopback port with the fixture symbols and a feed of the test process connected to it.
fn end_to_end(time_and_sale: bool, flags: &[&str]) -> (Outcome, Collected) {
    nano_time();
    let fixture = write_fixture(&format!("e2e-{time_and_sale}-{}.ipf", flags.join("")));
    let port = free_port();
    let address = format!(":{port}");
    let symbols = format!("5@{}", fixture.0.display());
    let start_millis = now_millis();
    let mut args = vec![
        "p",
        &address,
        "-S",
        &symbols,
        "-r",
        "*/2k",
        "-s",
        "1s",
        "--duration",
        "5s",
        "-Ddxscheme.nanoTime=true",
    ];
    args.extend_from_slice(flags);
    // The feed is subscribed before the tool starts: on the macOS x86-64 runner of CI the first endpoint of a process
    // takes about 5 s to create, as long as the tool runs.
    let collected = Arc::new(Mutex::new(Collected::default()));
    let feed = DXEndpoint::create_with_role(Role::Feed).unwrap();
    let sink = Arc::clone(&collected);
    let close_subscription: Box<dyn FnOnce()> = if time_and_sale {
        let subscription = feed.get_feed().create_subscription::<TimeAndSale>().unwrap();
        subscription
            .add_event_listener(move |sales| {
                let mut sink = sink.lock().unwrap();
                for sale in sales {
                    sink.add(&sale.symbol(), sale.index(), sale.time_nano_part(), start_millis);
                }
            })
            .unwrap();
        subscription.add_symbols(FIXTURE_SYMBOLS).unwrap();
        Box::new(move || subscription.close().unwrap())
    } else {
        let subscription = feed.get_feed().create_subscription::<Trade>().unwrap();
        subscription
            .add_event_listener(move |trades| {
                let mut sink = sink.lock().unwrap();
                for trade in trades {
                    sink.add(
                        &trade.symbol(),
                        trade.time_sequence(),
                        trade.time_nano_part(),
                        start_millis,
                    );
                }
            })
            .unwrap();
        subscription.add_symbols(FIXTURE_SYMBOLS).unwrap();
        Box::new(move || subscription.close().unwrap())
    };
    let tool = Tool::start(&args);
    // The port accepts once the tool listens.
    let deadline = Instant::now() + Duration::from_secs(30);
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(Instant::now() < deadline, "the port {port} does not accept");
        std::thread::sleep(Duration::from_millis(50));
    }
    feed.connect(&format!("127.0.0.1:{port}")).unwrap();
    let outcome = tool.finish();
    close_subscription();
    feed.close_and_await_termination().unwrap();
    let collected = std::mem::take(&mut *collected.lock().unwrap());
    (outcome, collected)
}

#[track_caller]
fn check_end_to_end(outcome: &Outcome, collected: &Collected) {
    assert_eq!(outcome.code(), Some(0), "{outcome:#?}");
    assert!(collected.events >= 100, "{collected:?} {outcome:#?}");
    assert_eq!(collected.foreign_symbols, 0, "{collected:?}");
    assert_eq!(collected.stale_time, 0, "{collected:?}");
    assert_eq!(collected.bad_nano_part, 0, "{collected:?}");
    // A nanosecond part of 0 in every event would mean that dxscheme.nanoTime did not reach the wire.
    assert!(collected.with_nano_part > 0, "{collected:?}");
    let summaries = outcome.summaries();
    assert!(summaries.len() >= 2, "{outcome:#?}");
    assert!(
        summaries.iter().all(|line| line.contains("instances 1")),
        "{outcome:#?}"
    );
    let connected = summaries.iter().filter(|line| line.contains("connections 1")).count();
    assert!(connected >= 2, "{outcome:#?}");
}

#[test]
fn a_feed_receives_the_published_trades_with_the_nanosecond_part() {
    let (outcome, collected) = end_to_end(false, &[]);
    check_end_to_end(&outcome, &collected);
}

#[test]
fn a_feed_receives_the_published_time_and_sales() {
    let (outcome, collected) = end_to_end(true, &["--event", "TimeAndSale"]);
    check_end_to_end(&outcome, &collected);
}

#[test]
fn a_feed_receives_the_reused_events() {
    let (outcome, collected) = end_to_end(false, &["--reuse"]);
    check_end_to_end(&outcome, &collected);
}

/// SIGINT ends a run without `--duration` with 0 and a last block.
#[cfg(unix)]
#[test]
fn sigint_ends_the_run() {
    // The period of a minute: the whole run is one block unless a minute boundary falls into it, so start early in a
    // minute.
    let second = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        % 60;
    if second > 45 {
        std::thread::sleep(Duration::from_secs(61 - second));
    }
    let port = free_port();
    let address = format!(":{port}");
    let tool = Tool::start(&["p", &address, "-S", "100", "-r", "*/5k", "-s", "1m"]);
    // The port accepts once the tool listens, after it has installed the Ctrl+C handler; then the run lasts
    // well over the second that the last block needs.
    let deadline = Instant::now() + Duration::from_secs(30);
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(Instant::now() < deadline, "the port {port} does not accept");
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_secs(3));
    let killed = Command::new("kill")
        .args(["-INT", &tool.child.id().to_string()])
        .status()
        .unwrap();
    assert!(killed.success());
    let outcome = tool.finish();
    assert_eq!(outcome.code(), Some(0), "{outcome:#?}");
    // One block, or two when a minute boundary fell into the run; the last one comes from the stop.
    let summaries = outcome.summaries();
    assert!(!summaries.is_empty() && summaries.len() <= 2, "{outcome:#?}");
    assert!(
        Regex::new("data [1-9][0-9,]* rps")
            .unwrap()
            .is_match(summaries[summaries.len() - 1]),
        "{outcome:#?}"
    );
}
