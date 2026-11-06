//! The LatencyTest tool: measures the delays between the times of quotes, trades and sales and the moments they come,
//! and prints their statistics every interval, as the LatencyTest tool of the C++ API does.

use std::collections::HashSet;
use std::error::Error;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dxfeed_graal::event::EventRef;
use dxfeed_graal::{DXEndpoint, Role, system};

use crate::args::{self, Arg, Parsed, optional, required};
use crate::console;
use crate::help::Info;
use crate::metrics;
use crate::stats::{self, Stats};

pub const INFO: Info = Info {
    name: "LatencyTest",
    short_description: "Connects to the specified address(es) and calculates latency.",
    description: "\nConnects to the specified address(es) and calculates latency.\n",
    usage: &["LatencyTest <address> <types> <symbols> [<options>]"],
    additional_info: &[],
    args: &[
        required(Arg::Address),
        optional(Arg::Types),
        optional(Arg::Symbols),
        optional(Arg::Properties),
        optional(Arg::ForceStream),
        optional(Arg::Interval),
        optional(Arg::IgnoreExchanges),
        optional(Arg::Help),
    ],
};

/// The measurement interval when none or no valid one is given.
const DEFAULT_INTERVAL: u64 = 2;

/// The measurement interval in seconds: the given whole number, or two seconds for none, zero or an invalid one.
pub fn interval(value: Option<&str>) -> u64 {
    value
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|&seconds| seconds > 0)
        .unwrap_or(DEFAULT_INTERVAL)
}

/// The exchange codes of a comma-separated list, each of one character, with the error message of the C++ tool; the
/// C++ tool counts bytes, so it takes only ASCII codes, while this takes any character.
pub fn exchange_codes(list: Option<&str>) -> Result<HashSet<char>, String> {
    let mut codes = HashSet::new();
    for code in list
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|code| !code.is_empty())
    {
        let mut chars = code.chars();
        match (chars.next(), chars.next()) {
            (Some(char), None) => {
                codes.insert(char);
            }
            _ => {
                return Err(format!(
                    "\"{}\" argument parsing error. \"{code}\" is not a one-character exchange code.",
                    Arg::IgnoreExchanges.name()
                ));
            }
        }
    }
    Ok(codes)
}

/// The delays and the symbols of the events since the previous measurements.
struct Diagnostic {
    interval: u64,
    ignored: HashSet<char>,
    events: AtomicUsize,
    state: Mutex<State>,
    /// Whether the measurements have stopped; the timer prints under this lock, so the tool ends after a whole table.
    stopped: Mutex<bool>,
}

struct State {
    started: Instant,
    measured: Instant,
    deltas: Vec<i64>,
    symbols: HashSet<String>,
}

impl Diagnostic {
    fn new(interval: u64, ignored: HashSet<char>) -> Diagnostic {
        let now = Instant::now();
        Diagnostic {
            interval,
            ignored,
            events: AtomicUsize::new(0),
            state: Mutex::new(State {
                started: now,
                measured: now,
                deltas: Vec::new(),
                symbols: HashSet::new(),
            }),
            stopped: Mutex::new(false),
        }
    }

    /// Prints the statistics unless the measurements have stopped.
    fn print(&self) {
        let stopped = self.stopped.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if !*stopped {
            console::print(&self.measure());
        }
    }

    /// Stops the measurements, waiting for a table being printed.
    fn stop(&self) {
        *self.stopped.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
    }

    fn is_ignored(&self, code: char) -> bool {
        self.ignored.contains(&code)
    }

    /// Takes the delay and the symbol of each quote, trade and new valid sale whose exchanges are not ignored.
    fn handle<'a>(&self, events: impl Iterator<Item = EventRef<'a>>) {
        let now = metrics::now_millis();
        let mut state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut valid = 0;
        for event in events {
            let (time, symbol) = match event {
                EventRef::Quote(quote)
                    if !self.is_ignored(quote.bid_exchange_code()) && !self.is_ignored(quote.ask_exchange_code()) =>
                {
                    (quote.time(), quote.symbol())
                }
                EventRef::Trade(trade) if !self.is_ignored(trade.exchange_code()) => (trade.time(), trade.symbol()),
                EventRef::TradeETH(trade) if !self.is_ignored(trade.exchange_code()) => (trade.time(), trade.symbol()),
                EventRef::TimeAndSale(sale)
                    if sale.is_new() && sale.is_valid_tick() && !self.is_ignored(sale.exchange_code()) =>
                {
                    (sale.time(), sale.symbol())
                }
                _ => continue,
            };
            valid += 1;
            state.deltas.push(now.wrapping_sub(time));
            if !state.symbols.contains(symbol.as_ref()) {
                state.symbols.insert(symbol.into_owned());
            }
        }
        self.events.fetch_add(valid, Ordering::Relaxed);
    }

    /// Takes the statistics since the previous measurements and returns their table.
    fn measure(&self) -> String {
        // The listener waits only while the sample is taken out; the statistics are computed without the lock.
        let (events_rate, mut deltas, symbols, started) = {
            let mut state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let events_rate = self.events.swap(0, Ordering::Relaxed) as f64 / state.measured.elapsed().as_secs_f64();
            let deltas = std::mem::take(&mut state.deltas);
            let symbols = state.symbols.len();
            state.symbols.clear();
            state.measured = Instant::now();
            (events_rate, deltas, symbols, state.started)
        };
        let stats = Stats::of(&mut deltas);
        format!(
            "\n{}\n{}\n\
             \x20 Rate of events (avg)     : {} (events/s)\n\
             \x20 Rate of unique symbols   : {symbols} (symbols/interval)\n\
             \x20 Min                      : {} (ms)\n\
             \x20 Max                      : {} (ms)\n\
             \x20 99th percentile          : {} (ms)\n\
             \x20 Mean                     : {} (ms)\n\
             \x20 StdDev                   : {} (ms)\n\
             \x20 Error                    : {} (ms)\n\
             \x20 Sample size (N)          : {} (events)\n\
             \x20 Measurement interval     : {} (s)\n\
             \x20 Running time             : {}\n",
            metrics::platform_info(),
            "-".repeat(52),
            stats::format(events_rate),
            stats::format(stats.min),
            stats::format(stats.max),
            stats::format(stats.percentile),
            stats::format(stats.mean),
            stats::format(stats.std_dev),
            stats::format(stats.std_err),
            stats.size,
            self.interval,
            metrics::hms(started.elapsed()),
        )
    }
}

/// Measures the delays of the events and prints their statistics until the address has no more events.
pub fn run(args: &Parsed) -> Result<(), Box<dyn Error>> {
    let ignored = exchange_codes(args.get(Arg::IgnoreExchanges))?;
    let interval = interval(args.get(Arg::Interval));
    let properties = args::parse_properties(args.get(Arg::Properties));
    for (key, value) in &properties {
        system::set_property(key, value)?;
    }
    let types = args::event_types(args.get(Arg::Types))?;
    let symbols = args::symbols(args.get(Arg::Symbols))?;

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
    let subscription = endpoint.get_feed().create_subscription_with_types(&types)?;
    let diagnostic = Arc::new(Diagnostic::new(interval, ignored));
    {
        let diagnostic = Arc::clone(&diagnostic);
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(interval));
                diagnostic.print();
            }
        });
    }
    {
        let diagnostic = Arc::clone(&diagnostic);
        subscription.add_event_listener(move |events| diagnostic.handle(events))?;
    }
    subscription.add_symbols(symbols)?;
    endpoint.connect(args.get(Arg::Address).unwrap_or_default())?;
    endpoint.await_not_connected()?;
    endpoint.close_and_await_termination()?;
    diagnostic.stop();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_interval_falls_back_to_two_seconds() {
        assert_eq!(interval(None), 2);
        assert_eq!(interval(Some("5")), 5);
        assert_eq!(interval(Some(" 7 ")), 7);
        assert_eq!(interval(Some("0")), 2);
        assert_eq!(interval(Some("-1")), 2);
        assert_eq!(interval(Some("soon")), 2);
    }

    #[test]
    fn exchange_codes_of_one_character() {
        assert_eq!(exchange_codes(None), Ok(HashSet::new()));
        assert_eq!(exchange_codes(Some("Q, N,,Z ")), Ok(HashSet::from(['Q', 'N', 'Z'])));
        assert_eq!(
            exchange_codes(Some("Q,NY")),
            Err("\"ignoreExchanges\" argument parsing error. \"NY\" is not a one-character exchange code.".to_owned())
        );
    }

    #[test]
    fn a_table_without_events() {
        let diagnostic = Diagnostic::new(3, HashSet::new());
        std::thread::sleep(Duration::from_millis(20));
        let table = diagnostic.measure();
        let lines: Vec<&str> = table.lines().collect();
        assert_eq!(lines[0], "");
        assert_eq!(lines[1], metrics::platform_info());
        assert_eq!(lines[2], "-".repeat(52));
        assert_eq!(lines[3], "  Rate of events (avg)     : 0.00 (events/s)", "{table}");
        assert_eq!(lines[4], "  Rate of unique symbols   : 0 (symbols/interval)", "{table}");
        assert_eq!(lines[5], "  Min                      : --- (ms)", "{table}");
        assert_eq!(lines[11], "  Sample size (N)          : 0 (events)", "{table}");
        assert_eq!(lines[12], "  Measurement interval     : 3 (s)", "{table}");
        assert!(
            lines[13].starts_with("  Running time             : 00:00:00."),
            "{table}"
        );
        assert_eq!(lines.len(), 14, "{table}");
    }
}
