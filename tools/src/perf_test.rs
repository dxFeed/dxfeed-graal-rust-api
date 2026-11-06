//! The PerfTest tool: counts the events of a subscription and prints their rates, the memory and the CPU usage of the
//! process every two seconds, as the PerfTest tool of the C++ API does.

use std::error::Error;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dxfeed_graal::{DXEndpoint, Role, system};

use crate::args::{self, Arg, Parsed, optional, required};
use crate::console;
use crate::help::Info;
use crate::metrics;

pub const INFO: Info = Info {
    name: "PerfTest",
    short_description: "Connects to specified address and calculates performance counters.",
    description: "\nConnects to the specified address(es) and calculates performance counters (events per second, cpu \
                  usage, etc).\n",
    usage: &["PerfTest <address> <types> <symbols> [<options>]"],
    additional_info: &[],
    args: &[
        required(Arg::Address),
        required(Arg::Types),
        required(Arg::Symbols),
        optional(Arg::Properties),
        optional(Arg::ForceStream),
        optional(Arg::CpuUsageByCore),
        optional(Arg::DetachListener),
        optional(Arg::Help),
    ],
};

/// The property that selects the output: `csv` for CSV lines, anything else for tables.
const FORMAT_PROPERTY: &str = "PerfTest.format";
/// The property with the seconds after which the symbols are removed and the subscription is closed.
const UNSUBSCRIBE_AFTER_PROPERTY: &str = "PerfTest.unsubscribeAfter";
/// The period of the measurements.
const PERIOD: Duration = Duration::from_secs(2);

/// The header of the CSV output.
pub const CSV_HEADER: &str = "Timestamp,Rate of events (avg) [events/s],Rate of listener calls [calls/s],Number of \
                              events in call (avg) [events],Current memory usage [MByte],Peak memory usage [MByte],\
                              Current CPU usage [%],Peak CPU usage [%],Running time,Entity.Event";

/// The share of the CPU that `cpu` time used over `wall` time on `cores` cores; NaN or infinity for no wall time.
pub fn cpu_usage(cpu: Duration, wall: Duration, cores: usize) -> f64 {
    cpu.as_secs_f64() / (wall.as_secs_f64() * cores as f64)
}

/// A rate with two decimals, NaN as zero.
fn rate(value: f64) -> String {
    format!("{:.2}", if value.is_nan() { 0.0 } else { value })
}

/// The counters of the listener and the measurements of the timer.
pub struct Diagnostic {
    cpu_usage_by_core: bool,
    csv: bool,
    events: AtomicUsize,
    listener_calls: AtomicUsize,
    state: Mutex<State>,
    /// Whether the measurements have stopped; the timer prints under this lock, so nothing follows the hash.
    stopped: Mutex<bool>,
}

struct State {
    started: Instant,
    measured: Instant,
    cpu_time: Duration,
    peak_memory: f64,
    peak_cpu: f64,
}

impl Diagnostic {
    pub fn new(cpu_usage_by_core: bool, csv: bool) -> Diagnostic {
        let now = Instant::now();
        Diagnostic {
            cpu_usage_by_core,
            csv,
            events: AtomicUsize::new(0),
            listener_calls: AtomicUsize::new(0),
            state: Mutex::new(State {
                started: now,
                measured: now,
                cpu_time: metrics::cpu_time().unwrap_or_default(),
                peak_memory: 0.0,
                peak_cpu: 0.0,
            }),
            stopped: Mutex::new(false),
        }
    }

    /// Prints the measurements unless they have stopped.
    pub fn print(&self) {
        let stopped = self.stopped.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if !*stopped {
            console::print(&self.measure());
        }
    }

    /// Stops the measurements and prints `last` as the last line.
    pub fn stop(&self, last: &str) {
        let mut stopped = self.stopped.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        *stopped = true;
        console::println(last);
    }

    /// Counts a call of the listener with `events` events.
    pub fn add_call(&self, events: usize) {
        self.listener_calls.fetch_add(1, Ordering::Relaxed);
        self.events.fetch_add(events, Ordering::Relaxed);
    }

    /// Takes the measurements since the previous ones and returns the table or the CSV line of them.
    ///
    /// The rates are per second of the measured interval; the C++ tool divides by its whole seconds.
    pub fn measure(&self) -> String {
        let mut state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let elapsed = state.measured.elapsed();
        let seconds = elapsed.as_secs_f64();
        let events_rate = self.events.swap(0, Ordering::Relaxed) as f64 / seconds;
        let calls_rate = self.listener_calls.swap(0, Ordering::Relaxed) as f64 / seconds;

        let memory = metrics::memory().map_or(0.0, |bytes| bytes as f64 / 1024.0 / 1024.0);
        state.peak_memory = state.peak_memory.max(memory);
        let cpu_time = metrics::cpu_time().unwrap_or(state.cpu_time);
        let cores = if self.cpu_usage_by_core { 1 } else { metrics::cores() };
        let cpu = cpu_usage(cpu_time.saturating_sub(state.cpu_time), elapsed, cores);
        state.cpu_time = cpu_time;
        state.peak_cpu = state.peak_cpu.max(cpu);
        let running = metrics::hms(state.started.elapsed());
        state.measured = Instant::now();

        let in_call = events_rate / calls_rate;
        if self.csv {
            return format!(
                "{},{},{},{},{memory:.3},{:.3},{:.2},{:.2},{running},{}\n",
                metrics::local_timestamp(metrics::now_millis()),
                rate(events_rate),
                rate(calls_rate),
                rate(in_call),
                state.peak_memory,
                cpu * 100.0,
                state.peak_cpu * 100.0,
                // The live events that the C++ tool counts only in its builds with metrics, and so prints as 0.
                0
            );
        }
        let separator = "-".repeat(52);
        format!(
            "\n{}\n{separator}\n\
             \x20 Rate of events (avg)           : {} (events/s)\n\
             \x20 Rate of listener calls         : {} (calls/s)\n\
             \x20 Number of events in call (avg) : {} (events)\n\
             \x20 Current memory usage           : {memory:.3} (MByte)\n\
             \x20 Peak memory usage              : {:.3} (MByte)\n\
             \x20 Current CPU usage              : {:.2} %\n\
             \x20 Peak CPU usage                 : {:.2} %\n\
             \x20 Running time                   : {running}\n\
             {separator}\n",
            metrics::platform_info(),
            rate(events_rate),
            rate(calls_rate),
            rate(in_call),
            state.peak_memory,
            cpu * 100.0,
            state.peak_cpu * 100.0,
        )
    }
}

/// The last value of `key` among `properties`.
fn property<'a>(properties: &'a [(String, String)], key: &str) -> Option<&'a str> {
    properties
        .iter()
        .rev()
        .find(|(k, _)| k == key)
        .map(|(_, value)| value.as_str())
}

/// Counts the events and prints the measurements until the address has no more events.
pub fn run(args: &Parsed) -> Result<(), Box<dyn Error>> {
    let properties = args::parse_properties(args.get(Arg::Properties));
    let csv = property(&properties, FORMAT_PROPERTY).is_some_and(|format| format.eq_ignore_ascii_case("csv"));
    let unsubscribe_after = match property(&properties, UNSUBSCRIBE_AFTER_PROPERTY) {
        Some(seconds) => seconds
            .trim()
            .parse::<u64>()
            .map_err(|_| format!("Invalid {UNSUBSCRIBE_AFTER_PROPERTY}: {seconds}"))?,
        None => 0,
    };
    for (key, value) in &properties {
        system::set_property(key, value)?;
    }
    if csv {
        console::println(CSV_HEADER);
    }
    let types = args::event_types(args.get(Arg::Types))?;
    let symbols = args::symbols(args.get(Arg::Symbols))?;
    console::println(&format!("Parsed symbols: {}", symbols.len()));

    let role = if args.has(Arg::ForceStream) {
        Role::StreamFeed
    } else {
        Role::Feed
    };
    let endpoint = DXEndpoint::new_builder()
        .with_role(role)
        .with_property(DXEndpoint::DXFEED_WILDCARD_ENABLE_PROPERTY, "true")
        .with_properties(properties.iter().map(|(key, value)| (key, value)))
        .with_name(&format!("{}Tool-Feed", INFO.name))
        .build()?;
    let subscription = Arc::new(endpoint.get_feed().create_subscription_with_types(&types)?);
    let diagnostic = Arc::new(Diagnostic::new(args.has(Arg::CpuUsageByCore), csv));
    {
        let diagnostic = Arc::clone(&diagnostic);
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(PERIOD);
                diagnostic.print();
            }
        });
    }

    // The listener folds every event into a hash, as the C++ tool hashes the event pointers, so that the events are
    // read as a real listener reads them; an event adds one plus its event time.
    let hash = Arc::new(AtomicU64::new(0));
    if !args.has(Arg::DetachListener) {
        let diagnostic = Arc::clone(&diagnostic);
        let hash = Arc::clone(&hash);
        subscription.add_event_listener(move |events| {
            diagnostic.add_call(events.len());
            let sum = events.fold(0u64, |sum, event| {
                sum.wrapping_add(event.event_time().unwrap_or_default() as u64)
                    .wrapping_add(1)
            });
            hash.fetch_add(std::hint::black_box(sum), Ordering::Relaxed);
        })?;
    }

    subscription.add_symbols(symbols.iter().cloned())?;
    endpoint.connect(args.get(Arg::Address).unwrap_or_default())?;
    if unsubscribe_after > 0 {
        let subscription = Arc::clone(&subscription);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(unsubscribe_after));
            let closed = subscription.remove_symbols(symbols).and_then(|()| subscription.close());
            if let Err(error) = closed {
                eprintln!("{error}");
            }
        });
    }

    endpoint.await_not_connected()?;
    endpoint.close_and_await_termination()?;
    diagnostic.stop(&hash.load(Ordering::Relaxed).to_string());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cpu_usage_of_all_cores_or_of_one() {
        let usage = cpu_usage(Duration::from_secs(4), Duration::from_secs(2), 8);
        assert!((usage - 0.25).abs() < 1e-9, "{usage}");
        let usage = cpu_usage(Duration::from_secs(1), Duration::from_secs(2), 1);
        assert!((usage - 0.5).abs() < 1e-9, "{usage}");
        assert!(cpu_usage(Duration::ZERO, Duration::ZERO, 1).is_nan());
    }

    #[test]
    fn nan_rates_are_zero() {
        assert_eq!(rate(f64::NAN), "0.00");
        assert_eq!(rate(12.345), "12.35");
    }

    #[test]
    fn a_table_of_the_measurements() {
        let diagnostic = Diagnostic::new(false, false);
        diagnostic.add_call(10);
        diagnostic.add_call(30);
        std::thread::sleep(Duration::from_millis(50));
        let table = diagnostic.measure();
        let lines: Vec<&str> = table.lines().collect();
        assert_eq!(lines[0], "");
        assert_eq!(lines[1], metrics::platform_info());
        assert_eq!(lines[2], "-".repeat(52));
        assert!(lines[3].starts_with("  Rate of events (avg)           : "), "{table}");
        assert!(
            lines[5].starts_with("  Number of events in call (avg) : 20.00 (events)"),
            "{table}"
        );
        assert!(
            lines[6].ends_with(" (MByte)") && !lines[6].contains(": 0.000"),
            "{table}"
        );
        assert!(
            lines[10].starts_with("  Running time                   : 00:00:00"),
            "{table}"
        );
        assert_eq!(lines[11], "-".repeat(52));
        assert_eq!(lines.len(), 12, "{table}");
        // The counters start again from zero.
        assert!(diagnostic.measure().contains(": 0.00 (events/s)"));
    }

    #[test]
    fn a_csv_line_of_the_measurements() {
        let diagnostic = Diagnostic::new(true, true);
        diagnostic.add_call(3);
        std::thread::sleep(Duration::from_millis(20));
        let line = diagnostic.measure();
        let fields: Vec<&str> = line.trim_end().split(',').collect();
        assert_eq!(fields.len(), CSV_HEADER.split(',').count(), "{line}");
        assert_eq!(fields[3], "3.00", "{line}");
        assert!(fields[8].starts_with("00:00:00."), "{line}");
    }

    #[test]
    fn the_last_property_wins() {
        let properties = vec![
            ("PerfTest.format".to_owned(), "table".to_owned()),
            ("PerfTest.format".to_owned(), "CSV".to_owned()),
        ];
        assert_eq!(property(&properties, FORMAT_PROPERTY), Some("CSV"));
        assert_eq!(property(&properties, UNSUBSCRIBE_AFTER_PROPERTY), None);
    }
}
