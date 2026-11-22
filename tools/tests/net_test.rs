//! The NetTest tool as a child process; the network is loopback only.

use std::io::{Read, Write};
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

fn fixture_text() -> String {
    let mut text = "#STOCK::=TYPE,SYMBOL,DESCRIPTION\n".to_owned();
    for symbol in FIXTURE_SYMBOLS {
        text.push_str(&format!("STOCK,{symbol},{symbol} Inc\n"));
    }
    text.push_str("##COMPLETE\n");
    text
}

fn write_fixture(name: &str) -> TempFile {
    let file = TempFile(std::env::temp_dir().join(format!("dxfeed-graal-tools-{}-{name}", std::process::id())));
    std::fs::write(&file.0, fixture_text()).unwrap();
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
        Tool::start_with_env(args, &[])
    }

    fn start_with_env(args: &[&str], variables: &[(&str, &str)]) -> Tool {
        let mut child = Command::new(env!("CARGO_BIN_EXE_Tools"))
            .arg("NetTest")
            .args(args)
            .envs(variables.iter().copied())
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
    // A free loopback port: nothing listens on it, so the connection count stays 0.
    let address = format!("127.0.0.1:{}", free_port());
    // The first tick only starts the count and a last block under a second is not printed, so 2 seconds left one
    // printed block that a late wake-up on a busy runner lost; 3 seconds leave a second of margin.
    let outcome = run(&["p", &address, "-S", "100", "-r", "*/5k", "-s", "1s", "--duration", "3s"]);
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
            .starts_with("NetTest: unknown flag --no-such-flag\n\nUsage: Tools NetTest p|c <address> -S <symbols>"),
        "{outcome:#?}"
    );
    assert!(outcome.stdout.is_empty(), "{outcome:#?}");
    assert!(outcome.stderr.ends_with("\nspec_version nettest/1\n"), "{outcome:#?}");

    let legacy = run(&["p", "127.0.0.1:1", "-S", "1", "--legacy"]);
    assert_eq!(legacy.code(), Some(2), "{legacy:#?}");
    assert!(
        legacy.stderr.starts_with("NetTest: unsupported option: --legacy"),
        "{legacy:#?}"
    );
}

#[test]
fn no_arguments_print_the_usage() {
    let outcome = run(&[]);
    assert_eq!(outcome.code(), Some(2), "{outcome:#?}");
    assert!(
        outcome
            .stdout
            .starts_with("Usage: Tools NetTest p|c <address> -S <symbols> [options]\n\n"),
        "{outcome:#?}"
    );
    assert!(outcome.stdout.ends_with("\nspec_version nettest/1\n"), "{outcome:#?}");
    assert_eq!(outcome.stderr, "", "{outcome:#?}");
}

#[test]
fn help_prints_the_usage_to_stdout_and_exits_with_zero() {
    for args in [
        &["--help"][..],
        &["-h"],
        &["-help"],
        &["p", "127.0.0.1:1", "-S", "10", "--help"],
        &["p", "127.0.0.1:1", "-S", "abc", "--help"],
        &["c", "--help"],
        &["c", "--no-such-flag", "-h"],
    ] {
        let outcome = run(args);
        assert_eq!(outcome.code(), Some(0), "{args:?} {outcome:#?}");
        assert!(
            outcome
                .stdout
                .starts_with("Usage: Tools NetTest p|c <address> -S <symbols> [options]\n\n"),
            "{outcome:#?}"
        );
        assert_eq!(
            outcome.stdout.lines().last(),
            Some("spec_version nettest/1"),
            "{outcome:#?}"
        );
        assert_eq!(outcome.stderr, "", "{outcome:#?}");
    }
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

#[test]
fn a_taken_server_port_fails_the_consumer_start() {
    let (taken, port) = take_wildcard_port();
    let address = format!(":{port}");
    let outcome = run(&["c", &address, "-S", "10", "--duration", "10s"]);
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
    let directory = TempDir::new("sigint");
    let file = directory.file("summary.json");
    let tool = Tool::start(&["p", &address, "-S", "100", "-r", "*/5k", "-s", "1m", "--summary", &file]);
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
    let summary = read_summary(&file);
    assert_eq!(summary["status"], "stopped_by_signal", "{summary}");
    assert_eq!(summary["exit_code"], 0);
    assert!(summary["records"].as_i64().unwrap() > 0, "{summary}");
}

/// Makes a FIFO as the symbol file: the tool waits in reading it until the test writes the symbols, as it would in a
/// large file.
#[cfg(unix)]
fn make_fifo(directory: &TempDir) -> String {
    let path = directory.file("symbols.ipf");
    let made = Command::new("mkfifo").arg(&path).status().unwrap();
    assert!(made.success(), "mkfifo {path}");
    path
}

/// Opens the FIFO for writing once the tool has opened it for reading, which it does with its signal handler up.
#[cfg(unix)]
fn open_fifo_writer(path: &str) -> std::fs::File {
    use std::os::unix::fs::OpenOptionsExt;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        // Without a reader a non-blocking open for writing fails.
        match std::fs::OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)
        {
            Ok(file) => return file,
            Err(error) => {
                assert!(Instant::now() < deadline, "NetTest does not open {path}: {error}");
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

#[cfg(unix)]
fn send_signal(tool: &Tool, signal: &str) {
    let sent = Command::new("kill")
        .args([signal, &tool.child.id().to_string()])
        .status()
        .unwrap();
    assert!(sent.success(), "kill {signal}");
}

/// A signal while the symbols are read ends the tool with the summary and 0, without the endpoints.
#[cfg(unix)]
#[test]
fn a_signal_while_the_symbols_are_read_ends_with_the_summary() {
    let directory = TempDir::new("signal-symbols");
    let fifo = make_fifo(&directory);
    let file = directory.file("summary.json");
    let address = format!(":{}", free_port());
    let symbols = format!("5@{fifo}");
    let tool = Tool::start(&["p", &address, "-S", &symbols, "-s", "1s", "--summary", &file]);
    let mut writer = open_fifo_writer(&fifo);
    send_signal(&tool, "-TERM");
    // The reading does not check the stop: it ends with the symbols.
    std::thread::sleep(Duration::from_millis(300));
    writer.write_all(fixture_text().as_bytes()).unwrap();
    drop(writer);
    let outcome = tool.finish();
    assert_eq!(outcome.code(), Some(0), "{outcome:#?}");
    assert!(outcome.summaries().is_empty(), "{outcome:#?}");
    let summary = read_summary(&file);
    assert_eq!(summary["status"], "stopped_by_signal", "{summary}");
    assert_eq!(summary["exit_code"], 0, "{summary}");
}

/// A second signal ends the process at once, with the default action of the signal and without the summary.
#[cfg(unix)]
#[test]
fn a_second_signal_ends_the_process_at_once() {
    use std::os::unix::process::ExitStatusExt;

    let directory = TempDir::new("second-signal");
    let fifo = make_fifo(&directory);
    let file = directory.file("summary.json");
    let address = format!(":{}", free_port());
    let tool = Tool::start(&["p", &address, "-S", &format!("5@{fifo}"), "--summary", &file]);
    let writer = open_fifo_writer(&fifo);
    send_signal(&tool, "-INT");
    std::thread::sleep(Duration::from_millis(300));
    send_signal(&tool, "-INT");
    let outcome = tool.finish();
    drop(writer);
    assert_eq!(outcome.status.signal(), Some(libc::SIGINT), "{outcome:#?}");
    assert!(outcome.summaries().is_empty(), "{outcome:#?}");
    assert!(
        !std::path::Path::new(&file).exists(),
        "a summary after the second signal"
    );
}

/// Waits until something accepts connections on the loopback port.
fn wait_for_port(port: u16) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(Instant::now() < deadline, "the port {port} does not accept");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_consumer_receives_from_a_publisher_tool_and_exits_with_zero() {
    let port = free_port();
    let address = format!(":{port}");
    // The publisher outlives the consumer by far: the consumer's first endpoint may take seconds on a slow runner.
    let publisher = Tool::start(&["p", &address, "-S", "20", "-r", "*/2k", "-s", "1s", "--duration", "10s"]);
    wait_for_port(port);
    let client = format!("127.0.0.1:{port}");
    let consumer = run(&["c", &client, "-S", "20", "-s", "1s", "--duration", "3s"]);
    let publisher = publisher.finish();
    assert_eq!(publisher.code(), Some(0), "{publisher:#?}");
    assert_eq!(consumer.code(), Some(0), "{consumer:#?}");
    // Each block is a summary line, the instance line and the runtime line.
    let lines: Vec<&str> = consumer.stdout.lines().collect();
    assert!(lines.len() >= 3 && lines.len().is_multiple_of(3), "{consumer:#?}");
    for block in lines.chunks(3) {
        assert!(
            block[0].starts_with("{*NetTestConsumerSide-NetTest*} data "),
            "{block:?}"
        );
        assert!(block[1].starts_with("    NetTest.1 data "), "{block:?}");
        assert!(block[2].starts_with("{*NetTestRuntime-NetTest*} heap "), "{block:?}");
    }
    let summary = Regex::new(
        "^[{][*]NetTestConsumerSide-NetTest[*][}] data ([0-9,]+) rps, lag (-?[0-9,]+) us, instances 1, connections ([0-9]+)$",
    )
    .unwrap();
    let received = lines
        .iter()
        .filter_map(|line| summary.captures(line))
        .filter(|captures| &captures[1] != "0" && &captures[3] == "1")
        .count();
    assert!(received >= 1, "{consumer:#?}");
    // The isolate logs to stderr; the tool itself has nothing to say.
    assert!(!consumer.stderr.contains("NetTest:"), "{consumer:#?}");
}

#[test]
fn a_connected_consumer_without_records_ends_with_no_data() {
    // An endpoint of the test process accepts the connection and publishes nothing.
    let port = free_port();
    let endpoint = DXEndpoint::create_with_role(Role::Publisher).unwrap();
    endpoint.connect(&format!(":{port}")).unwrap();
    wait_for_port(port);
    let client = format!("127.0.0.1:{port}");
    let consumer = Tool::start_with_env(
        &["c", &client, "-S", "5", "-s", "1s", "--duration", "60s"],
        &[("DXFG_NETTEST_WATCHDOG", "500,1500")],
    )
    .finish();
    endpoint.close_and_await_termination().unwrap();
    assert_eq!(consumer.code(), Some(1), "{consumer:#?}");
    assert!(
        consumer
            .stderr
            .ends_with("NetTest: no_data: no records for 1500 ms after the connection, with 1 open connections\n"),
        "{consumer:#?}"
    );
}

/// A directory in the temporary directory, removed with everything in it when the test ends.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("dxfeed-graal-tools-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }

    /// A path in the directory as text, for the command line.
    fn file(&self, name: &str) -> String {
        self.0.join(name).to_str().unwrap().to_owned()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn read_summary(file: &str) -> serde_json::Value {
    let text = std::fs::read_to_string(file).unwrap_or_else(|error| panic!("no summary file {file}: {error}"));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("the summary file is not JSON: {error}\n{text}"))
}

#[test]
fn a_producer_writes_the_summary_file() {
    let directory = TempDir::new("summary-producer");
    let file = directory.file("summary.json");
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
        "--summary",
        &file,
    ]);
    assert_eq!(outcome.code(), Some(0), "{outcome:#?}");
    assert!(!std::path::Path::new(&format!("{file}.tmp")).exists());
    let summary = read_summary(&file);
    assert_eq!(summary["side"], "producer", "{summary}");
    assert_eq!(summary["tool"]["implementation"], "rust-api");
    // The SDK tells the QDS version and the runtime.
    assert_eq!(
        summary["tool"]["sdk_version"],
        dxfeed_graal_sys::SDK_VERSION,
        "{summary}"
    );
    assert!(
        summary["tool"]["qd_version"]
            .as_str()
            .is_some_and(|version| version.starts_with("3.")),
        "{summary}"
    );
    for key in ["vendor", "march", "gc"] {
        assert!(summary["tool"]["build"][key].is_string(), "{summary}");
    }
    assert_eq!(summary["status"], "ok");
    assert_eq!(summary["exit_code"], 0);
    assert_eq!(summary["error"], serde_json::Value::Null);
    assert_eq!(summary["schema"], "nettest-summary/1");
    assert_eq!(summary["config"]["rate_limit"], 5000);
    assert_eq!(summary["args"][0], "p");
    assert_eq!(summary["args"].as_array().unwrap().last().unwrap(), file.as_str());
    let intervals = summary["intervals"].as_array().unwrap();
    assert!(intervals.len() >= 3, "{summary}");
    let records: i64 = intervals
        .iter()
        .map(|interval| interval["records"].as_i64().unwrap())
        .sum();
    // 3 s at 5k per second: at least one second's worth, and not above the schedule plus a generous catch-up.
    assert!((5_000..=20_000).contains(&records), "{summary}");
    assert_eq!(summary["records"], records);
    assert_eq!(summary["publish_call"]["events"], records);
    assert!(summary["publish_call"]["calls"].as_i64().unwrap() > 0, "{summary}");
    assert!(summary["schedule_slip"]["max_ns"].is_i64() || summary["schedule_slip"]["max_ns"].is_u64());
    assert!(summary["e2e_lag"].is_null() && summary["symbols_seen"].is_null());
    // Every interval is a measurement, and the intervals follow each other.
    assert!(
        intervals.iter().all(|interval| interval["phase"] == "measure"),
        "{summary}"
    );
    for pair in intervals.windows(2) {
        assert_eq!(pair[0]["end_epoch_ms"], pair[1]["start_epoch_ms"], "{summary}");
    }
    assert_eq!(summary["connectors"][0]["address"], "127.0.0.1:1", "{summary}");
}

#[test]
fn a_taken_server_port_writes_a_summary_with_the_error() {
    let (taken, port) = take_wildcard_port();
    let directory = TempDir::new("summary-port");
    let file = directory.file("summary.json");
    let address = format!(":{port}");
    let outcome = run(&["p", &address, "-S", "10", "--duration", "10s", "--summary", &file]);
    drop(taken);
    assert_eq!(outcome.code(), Some(1), "{outcome:#?}");
    let summary = read_summary(&file);
    assert_eq!(summary["status"], "error", "{summary}");
    assert_eq!(summary["exit_code"], 1);
    let error = summary["error"].as_str().unwrap();
    assert!(error.contains("did not start in 3000 ms"), "{summary}");
    assert!(outcome.stderr.contains(&format!("NetTest: {error}\n")), "{outcome:#?}");
    assert_eq!(summary["intervals"], serde_json::json!([]));
    assert_eq!(summary["records"], 0);
}

#[test]
fn a_failure_before_the_run_writes_a_summary_with_the_error() {
    let directory = TempDir::new("summary-before-run");
    let file = directory.file("summary.json");
    let missing = directory.file("no-such-symbols.ipf");
    let symbols = format!("10@{missing}");
    let outcome = run(&["p", "127.0.0.1:1", "-S", &symbols, "--summary", &file]);
    assert_eq!(outcome.code(), Some(1), "{outcome:#?}");
    let summary = read_summary(&file);
    assert_eq!(summary["status"], "error", "{summary}");
    assert_eq!(summary["exit_code"], 1);
    assert_eq!(summary["config"]["symbols_source"], "file");
    assert!(
        summary["error"].as_str().unwrap().contains("no-such-symbols.ipf"),
        "{summary}"
    );
    assert_eq!(summary["intervals"], serde_json::json!([]));
}

#[test]
fn an_unwritable_summary_path_is_a_bad_command_line_without_a_summary() {
    let directory = TempDir::new("summary-unwritable");
    let file = directory.file("no-such-dir/summary.json");
    let outcome = run(&["p", "127.0.0.1:1", "-S", "10", "--duration", "1s", "--summary", &file]);
    assert_eq!(outcome.code(), Some(2), "{outcome:#?}");
    assert!(
        outcome.stderr.starts_with(&format!(
            "NetTest: --summary cannot write to the directory of '{file}'\n"
        )),
        "{outcome:#?}"
    );
    assert!(!std::path::Path::new(&file).exists());
}

#[test]
fn a_summary_that_cannot_be_written_at_the_end_exits_with_one() {
    let directory = TempDir::new("summary-blocked");
    let file = directory.file("summary.json");
    // The directory of the file is writable at the start; the temporary file of the end is a directory.
    std::fs::create_dir(format!("{file}.tmp")).unwrap();
    let outcome = run(&[
        "p",
        "127.0.0.1:1",
        "-S",
        "10",
        "-s",
        "1s",
        "--duration",
        "1s",
        "--summary",
        &file,
    ]);
    assert_eq!(outcome.code(), Some(1), "{outcome:#?}");
    assert!(
        outcome
            .stderr
            .contains(&format!("NetTest: cannot write the summary file '{file}': ")),
        "{outcome:#?}"
    );
    assert!(!std::path::Path::new(&file).exists());
}

#[test]
fn a_consumer_without_records_writes_a_no_data_summary() {
    let port = free_port();
    let endpoint = DXEndpoint::create_with_role(Role::Publisher).unwrap();
    endpoint.connect(&format!(":{port}")).unwrap();
    wait_for_port(port);
    let directory = TempDir::new("summary-consumer");
    let file = directory.file("summary.json");
    let client = format!("127.0.0.1:{port}");
    let consumer = Tool::start_with_env(
        &[
            "c",
            &client,
            "-S",
            "5",
            "-s",
            "1s",
            "--duration",
            "60s",
            "--summary",
            &file,
        ],
        &[("DXFG_NETTEST_WATCHDOG", "500,1500")],
    )
    .finish();
    endpoint.close_and_await_termination().unwrap();
    assert_eq!(consumer.code(), Some(1), "{consumer:#?}");
    let summary = read_summary(&file);
    assert_eq!(summary["side"], "consumer", "{summary}");
    assert_eq!(summary["status"], "no_data");
    assert_eq!(summary["exit_code"], 1);
    assert!(
        summary["error"]
            .as_str()
            .unwrap()
            .starts_with("no_data: no records for 1500 ms"),
        "{summary}"
    );
    assert_eq!(summary["records"], 0);
    assert_eq!(summary["e2e_lag"]["count"], 0);
    assert_eq!(summary["e2e_lag"]["p50_ns"], serde_json::Value::Null);
    assert_eq!(summary["symbols_seen"], 0);
    assert_eq!(summary["publish_call"], serde_json::Value::Null);
    assert_eq!(summary["connectors"].as_array().unwrap().len(), 1, "{summary}");
}

/// The e2e lag block of a consumer summary: the count and the mean in nanoseconds.
fn lag_of(summary: &serde_json::Value) -> (i64, i64) {
    let lag = &summary["e2e_lag"];
    (lag["count"].as_i64().unwrap(), lag["mean_ns"].as_i64().unwrap())
}

#[test]
fn two_consumer_instances_see_every_symbol_of_a_publisher_process() {
    let port = free_port();
    let address = format!(":{port}");
    let publisher = Tool::start(&[
        "p",
        &address,
        "-S",
        "100",
        "-r",
        "*/2k",
        "-s",
        "1s",
        "--duration",
        "12s",
    ]);
    wait_for_port(port);
    let directory = TempDir::new("pair-consumer");
    let file = directory.file("summary.json");
    let client = format!("127.0.0.1:{port}");
    let consumer = run(&[
        "c",
        &client,
        "-S",
        "100",
        "-C",
        "2",
        "-s",
        "1s",
        "--duration",
        "5s",
        "--summary",
        &file,
    ]);
    let publisher = publisher.finish();
    assert_eq!(publisher.code(), Some(0), "{publisher:#?}");
    assert_eq!(consumer.code(), Some(0), "{consumer:#?}");
    let both_connected = Regex::new(
        "[{][*]NetTestConsumerSide-NetTest[*][}] data [1-9][0-9,]* rps, lag -?[0-9,]+ us, instances 2, connections 2",
    )
    .unwrap();
    assert!(both_connected.is_match(&consumer.stdout), "{consumer:#?}");
    assert!(consumer.stdout.contains("\n    NetTest.2 data "), "{consumer:#?}");
    let summary = read_summary(&file);
    assert_eq!(summary["status"], "ok", "{summary}");
    assert_eq!(summary["symbols_seen"], 100, "{summary}");
    assert_eq!(summary["connectors"].as_array().unwrap().len(), 2, "{summary}");
    let (count, mean) = lag_of(&summary);
    assert!(count > 0, "{summary}");
    // Positive (the lag has the nanosecond part) and well below a second on loopback.
    assert!((1..1_000_000_000).contains(&mean), "{summary}");
    let intervals = summary["intervals"].as_array().unwrap();
    let records: i64 = intervals
        .iter()
        .map(|interval| interval["records"].as_i64().unwrap())
        .sum();
    assert_eq!(summary["records"], records, "{summary}");
}

#[test]
fn the_limiter_waits_ahead_so_a_batch_is_not_published_late() {
    // Spec test 7: with 50 events per call at 1k per second a batch is due every 50 ms; a limiter that sleeps after
    // the call would stamp events up to a batch late, a mean lag near 50 ms. The spec asks for a mean well below
    // 50 ms; the bound is half of it, 5 times the lag that loopback shows.
    let port = free_port();
    let address = format!(":{port}");
    let publisher = Tool::start(&[
        "p",
        &address,
        "-S",
        "100",
        "-r",
        "*/1k",
        "--batch",
        "50",
        "-s",
        "1s",
        "--duration",
        "12s",
    ]);
    wait_for_port(port);
    let directory = TempDir::new("pair-batch");
    let file = directory.file("summary.json");
    let client = format!("127.0.0.1:{port}");
    let consumer = run(&[
        "c",
        &client,
        "-S",
        "100",
        "-s",
        "1s",
        "--warmup",
        "1s",
        "--duration",
        "5s",
        "--summary",
        &file,
    ]);
    let publisher = publisher.finish();
    assert_eq!(publisher.code(), Some(0), "{publisher:#?}");
    assert_eq!(consumer.code(), Some(0), "{consumer:#?}");
    let (count, mean) = lag_of(&read_summary(&file));
    assert!(count > 100, "{consumer:#?}");
    assert!((1..25_000_000).contains(&mean), "mean lag {mean} ns\n{consumer:#?}");
}

#[test]
fn a_consumer_that_finds_no_nanosecond_part_ends_with_nano_missing() {
    let port = free_port();
    let address = format!(":{port}");
    let publisher = Tool::start(&[
        "p",
        &address,
        "-S",
        "100",
        "-r",
        "*/2k",
        "-s",
        "1s",
        "--duration",
        "10s",
        "-Ddxscheme.nanoTime=false",
    ]);
    wait_for_port(port);
    let directory = TempDir::new("pair-nano");
    let file = directory.file("summary.json");
    let client = format!("127.0.0.1:{port}");
    let consumer = run(&[
        "c",
        &client,
        "-S",
        "100",
        "-s",
        "1s",
        "--duration",
        "60s",
        "--summary",
        &file,
    ]);
    let publisher = publisher.finish();
    assert_eq!(publisher.code(), Some(0), "{publisher:#?}");
    assert_eq!(consumer.code(), Some(1), "{consumer:#?}");
    assert!(
        consumer
            .stderr
            .contains("NetTest: nano_missing: the first 1000 events have the nanosecond part 0"),
        "{consumer:#?}"
    );
    let summary = read_summary(&file);
    assert_eq!(summary["status"], "nano_missing", "{summary}");
    assert_eq!(summary["exit_code"], 1);
}

/// The answer of the metrics server to a request, read until it closes the connection.
fn exchange(port: u16, request: &str) -> String {
    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    client.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    client.write_all(request.as_bytes()).unwrap();
    let mut bytes = Vec::new();
    let _ = client.read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The answer to `GET /metrics`; a failed scrape (the server is not up yet) is an empty text.
fn scrape(port: u16) -> String {
    let Ok(mut client) = TcpStream::connect(("127.0.0.1", port)) else {
        return String::new();
    };
    let _ = client.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = client.write_all(b"GET /metrics HTTP/1.0\r\n\r\n");
    let mut bytes = Vec::new();
    let _ = client.read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Scrapes until the text satisfies the condition; the last text is the failure message.
fn scrape_until(port: u16, condition: impl Fn(&str) -> bool) -> String {
    let deadline = Instant::now() + Duration::from_secs(40);
    loop {
        let text = scrape(port);
        if condition(&text) {
            return text;
        }
        assert!(Instant::now() < deadline, "no such metrics in time:\n{text}");
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// The value of the first series that starts with the prefix.
fn series_value(text: &str, prefix: &str) -> Option<i64> {
    text.lines()
        .find_map(|line| line.strip_prefix(prefix))
        .and_then(|rest| rest.trim().parse().ok())
}

#[test]
fn a_producer_serves_its_metrics_during_the_run() {
    let port = free_port();
    let metrics = port.to_string();
    let directory = TempDir::new("metrics-producer");
    let file = directory.file("summary.json");
    let tool = Tool::start(&[
        "p",
        "127.0.0.1:1",
        "-S",
        "10",
        "-r",
        "*/2k",
        "-s",
        "1s",
        "--duration",
        "6s",
        "--metrics-port",
        &metrics,
        "--summary",
        &file,
    ]);
    wait_for_port(port);
    let labels = "tool=\"rust-api\",side=\"producer\",name=\"NetTest\"";
    // The published records are live; the runtime values come with the first tick of the reporter.
    let text = scrape_until(port, |text| {
        series_value(text, &format!("nettest_records_total{{{labels},connection=\"1\"}} ")).is_some_and(|n| n > 0)
            && text.contains(&format!("nettest_runtime_heap_used_bytes{{{labels}}} "))
    });
    assert!(text.starts_with("HTTP/1.0 200 OK\r\n"), "{text}");
    assert!(
        text.contains("\r\nContent-Type: text/plain; version=0.0.4; charset=utf-8\r\n"),
        "{text}"
    );
    assert!(
        text.contains(&format!("nettest_info{{{labels},spec_version=\"nettest/1\",version=\"")),
        "{text}"
    );
    for series in [
        "nettest_publish_calls_total",
        "nettest_publish_seconds_total",
        "nettest_schedule_slip_seconds",
        "nettest_connector_written_bytes_total",
    ] {
        assert!(text.contains(&format!("\n{series}{{{labels}}} ")), "{series}\n{text}");
    }
    assert!(
        text.contains(&format!("\nnettest_connections{{{labels},connection=\"1\"}} 0\n")),
        "{text}"
    );
    // The producer has no e2e lag and no connector lag.
    assert!(!text.contains("nettest_e2e_lag_seconds"), "{text}");
    assert!(!text.contains("nettest_connector_lag_seconds_total"), "{text}");
    assert!(exchange(port, "GET /other HTTP/1.0\r\n\r\n").starts_with("HTTP/1.0 404 Not Found\r\n"));
    let outcome = tool.finish();
    assert_eq!(outcome.code(), Some(0), "{outcome:#?}");
    let line = format!("{{*NetTestMetrics-NetTest*}} http://127.0.0.1:{port}/metrics");
    assert_eq!(outcome.stdout.lines().next(), Some(line.as_str()), "{outcome:#?}");
    assert_eq!(read_summary(&file)["status"], "ok");
    // The server is gone with the run.
    assert!(TcpStream::connect(("127.0.0.1", port)).is_err());
}

#[test]
fn port_zero_takes_a_free_port_and_the_line_names_it() {
    let outcome = run(&[
        "p",
        "127.0.0.1:1",
        "-S",
        "10",
        "--duration",
        "1s",
        "--metrics-port",
        "127.0.0.1:0",
    ]);
    assert_eq!(outcome.code(), Some(0), "{outcome:#?}");
    let line = Regex::new("^[{][*]NetTestMetrics-NetTest[*][}] http://127[.]0[.]0[.]1:([0-9]+)/metrics$").unwrap();
    let first = outcome.stdout.lines().next().unwrap_or_default();
    let captures = line.captures(first).unwrap_or_else(|| panic!("{outcome:#?}"));
    assert_ne!(&captures[1], "0", "{outcome:#?}");
}

#[test]
fn a_consumer_serves_its_metrics_during_the_run() {
    let port = free_port();
    let address = format!(":{port}");
    let publisher = Tool::start(&["p", &address, "-S", "20", "-r", "*/2k", "-s", "1s", "--duration", "12s"]);
    wait_for_port(port);
    let metrics_port = free_port();
    let metrics = metrics_port.to_string();
    let client = format!("127.0.0.1:{port}");
    let consumer = Tool::start(&[
        "c",
        &client,
        "-S",
        "20",
        "-s",
        "1s",
        "--duration",
        "5s",
        "--metrics-port",
        &metrics,
    ]);
    wait_for_port(metrics_port);
    let labels = "tool=\"rust-api\",side=\"consumer\",name=\"NetTest\"";
    let text = scrape_until(metrics_port, |text| {
        series_value(text, &format!("nettest_e2e_lag_seconds_count{{{labels}}} ")).is_some_and(|n| n > 0)
            && series_value(text, &format!("nettest_connections{{{labels},connection=\"1\"}} ")) == Some(1)
    });
    assert!(
        text.contains(&format!("nettest_e2e_lag_seconds_bucket{{{labels},le=\"+Inf\"}} ")),
        "{text}"
    );
    assert!(
        text.contains(&format!("\nnettest_e2e_lag_negative_total{{{labels}}} ")),
        "{text}"
    );
    assert!(
        series_value(&text, &format!("nettest_records_total{{{labels},connection=\"1\"}} ")).is_some_and(|n| n > 0),
        "{text}"
    );
    // The consumer has no publish counters.
    assert!(!text.contains("nettest_publish_calls_total"), "{text}");
    let consumer = consumer.finish();
    assert_eq!(consumer.code(), Some(0), "{consumer:#?}");
    assert!(
        consumer.stdout.starts_with(&format!(
            "{{*NetTestMetrics-NetTest*}} http://127.0.0.1:{metrics_port}/metrics\n"
        )),
        "{consumer:#?}"
    );
    let publisher = publisher.finish();
    assert_eq!(publisher.code(), Some(0), "{publisher:#?}");
}

#[test]
fn a_second_tool_on_a_taken_metrics_port_fails_the_start_with_the_error_status() {
    let port = free_port();
    let metrics = port.to_string();
    let mut first = Tool::start(&[
        "p",
        "127.0.0.1:1",
        "-S",
        "10",
        "--duration",
        "60s",
        "--metrics-port",
        &metrics,
    ]);
    wait_for_port(port);
    let directory = TempDir::new("metrics-taken");
    let file = directory.file("summary.json");
    let second = run(&[
        "p",
        "127.0.0.1:1",
        "-S",
        "10",
        "--duration",
        "10s",
        "--metrics-port",
        &metrics,
        "--summary",
        &file,
    ]);
    let _ = first.child.kill();
    let _ = first.finish();
    assert_eq!(second.code(), Some(1), "{second:#?}");
    let error = format!("cannot listen on 127.0.0.1:{port} for /metrics (is the port in use?)");
    assert!(second.stderr.contains(&format!("NetTest: {error}\n")), "{second:#?}");
    assert!(!second.stdout.contains("NetTestMetrics"), "{second:#?}");
    let summary = read_summary(&file);
    assert_eq!(summary["status"], "error", "{summary}");
    assert_eq!(summary["exit_code"], 1);
    assert_eq!(summary["error"], error.as_str());
    assert_eq!(summary["intervals"], serde_json::json!([]));
}
