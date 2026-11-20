//! The consumer mode of NetTest (section "Consumer" of the specification): feed endpoints, each subscribed to its slice
//! of the symbols, a listener that measures the e2e lag of every event, the check of the nanosecond part and the
//! starvation watchdog that the main thread runs.

use std::borrow::Cow;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use dxfeed_graal::{DXEndpoint, DXFeedSubscription, Event, EventRef, EventType, Events, system};

use super::cli::{EventKind, Options};
use super::events::now_epoch_nanos;
use super::generator::Counters;
use super::limiter::Stop;
use super::statistics::{
    InstanceTotals, IntervalTaken, LagRecorder, ReporterOutcome, ReporterSettings, RuntimeSample, Side,
    StatisticsSample, run_reporter, system_clock,
};
use super::summary::{ConnectorSummary, RunReport, RunResult, connector_summaries, consumer_report};
use super::symbols::{Symbols, consumer_slice};
use super::{Status, connections_of};

/// The consumer stops with `nano_missing` when this many first events all have the nanosecond part 0.
pub const NANO_CHECK_EVENTS: i64 = 1000;
/// How often the main thread counts the connections for the watchdog.
const WATCHDOG_PERIOD: Duration = Duration::from_millis(200);
/// With open connections, no records for this long after the last record stops the run...
const NO_DATA_AFTER_RECORD: Duration = Duration::from_secs(30);
/// ...and so does no record at all for this long after the connection.
const NO_DATA_AFTER_CONNECT: Duration = Duration::from_secs(60);
/// The variable that shortens the watchdog for the process tests: `<after the last record ms>,<after the connection
/// ms>`. The tool documents no such variable.
pub const WATCHDOG_VARIABLE: &str = "DXFG_NETTEST_WATCHDOG";

/// The upper bounds of the buckets of `nettest_e2e_lag_seconds` in nanoseconds; the last bucket is `+Inf`.
pub const LAG_BUCKET_BOUNDS_NANOS: [i64; 13] = [
    50_000,
    100_000,
    250_000,
    500_000,
    1_000_000,
    2_500_000,
    5_000_000,
    10_000_000,
    25_000_000,
    50_000_000,
    100_000_000,
    250_000_000,
    1_000_000_000,
];

/// The e2e lag of an event in nanoseconds: `now` minus its time, which is packed with the sequence as the seconds in
/// bits 32 to 63 and the milliseconds in bits 22 to 31, plus the nanosecond part. May be negative.
pub fn event_lag_nanos(now_nanos: i64, packed_time: i64, nano_part: i32) -> i64 {
    let millis = (packed_time >> 32) * 1000 + ((packed_time >> 22) & 0x3FF);
    now_nanos - (millis * 1_000_000 + i64::from(nano_part))
}

/// The bucket of a lag: the first bound that is not below it; a negative lag falls into the first bucket.
pub fn lag_bucket_index(lag_nanos: i64) -> usize {
    LAG_BUCKET_BOUNDS_NANOS.partition_point(|&bound| bound < lag_nanos)
}

/// The monotonic clock of the watchdog in nanoseconds, never 0 (0 means "no record yet").
fn steady_nanos() -> i64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_nanos() as i64 + 1
}

/// The timeouts of the watchdog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WatchdogTimeouts {
    pub after_record: Duration,
    pub after_connect: Duration,
}

impl Default for WatchdogTimeouts {
    fn default() -> Self {
        WatchdogTimeouts {
            after_record: NO_DATA_AFTER_RECORD,
            after_connect: NO_DATA_AFTER_CONNECT,
        }
    }
}

impl WatchdogTimeouts {
    /// The timeouts of the value of `DXFG_NETTEST_WATCHDOG`; the defaults without a value or with one that is not
    /// `<ms>,<ms>`.
    pub fn from_variable(value: Option<&str>) -> Self {
        let parse = || {
            let (record, connect) = value?.split_once(',')?;
            Some(WatchdogTimeouts {
                after_record: Duration::from_millis(record.trim().parse().ok()?),
                after_connect: Duration::from_millis(connect.trim().parse().ok()?),
            })
        };
        parse().unwrap_or_default()
    }
}

/// The starvation watchdog: with open connections, no records for the timeout after the last record, or after the
/// connection when no record came at all. No verdict while nothing is connected.
pub struct Watchdog {
    timeouts: WatchdogTimeouts,
    /// The time since which connections are open, unset while there are none.
    connected_since: Option<i64>,
}

impl Watchdog {
    pub fn new(timeouts: WatchdogTimeouts) -> Self {
        Watchdog {
            timeouts,
            connected_since: None,
        }
    }

    /// The message of the starvation, or `None`. The times are on one monotonic clock in nanoseconds;
    /// `last_record_nanos` is 0 before the first record.
    pub fn check(&mut self, now_nanos: i64, connections: i64, last_record_nanos: i64) -> Option<String> {
        if connections <= 0 {
            self.connected_since = None;
            return None;
        }
        let connected_since = *self.connected_since.get_or_insert(now_nanos);
        let nanos = |timeout: Duration| i64::try_from(timeout.as_nanos()).unwrap_or(i64::MAX);
        if last_record_nanos == 0 {
            let timeout = self.timeouts.after_connect;
            return (now_nanos - connected_since >= nanos(timeout)).then(|| {
                format!(
                    "no records for {} ms after the connection, with {connections} open connections",
                    timeout.as_millis()
                )
            });
        }
        let timeout = self.timeouts.after_record;
        (now_nanos - last_record_nanos.max(connected_since) >= nanos(timeout)).then(|| {
            format!(
                "no records for {} ms after the last record, with {connections} open connections",
                timeout.as_millis()
            )
        })
    }
}

/// The check of the nanosecond part over the first events of all instances: when the first `NANO_CHECK_EVENTS` all
/// have the part 0 while `nanoTime` is on, a hop of the path lacks the property.
pub struct NanoCheck {
    enabled: bool,
    pending: AtomicBool,
    missing: AtomicBool,
    state: Mutex<NanoState>,
}

#[derive(Default)]
struct NanoState {
    checked: i64,
    seen: bool,
}

impl NanoCheck {
    pub fn new(enabled: bool) -> Self {
        NanoCheck {
            enabled,
            pending: AtomicBool::new(enabled),
            missing: AtomicBool::new(false),
            state: Mutex::new(NanoState::default()),
        }
    }

    /// Whether the events still need to be checked.
    pub fn is_pending(&self) -> bool {
        self.pending.load(Ordering::Relaxed)
    }

    pub fn is_missing(&self) -> bool {
        self.missing.load(Ordering::SeqCst)
    }

    /// Counts the nanosecond parts of the events of one listener call, in their order, up to `NANO_CHECK_EVENTS` in
    /// all; true when this call completed the check with every part 0.
    pub fn check(&self, nano_parts: &[i32]) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if !self.enabled || !self.pending.load(Ordering::SeqCst) {
            return false;
        }
        for &part in nano_parts {
            if state.checked >= NANO_CHECK_EVENTS {
                break;
            }
            state.checked += 1;
            state.seen |= part != 0;
        }
        if state.seen {
            self.pending.store(false, Ordering::SeqCst);
            false
        } else if state.checked >= NANO_CHECK_EVENTS {
            self.pending.store(false, Ordering::SeqCst);
            self.missing.store(true, Ordering::SeqCst);
            true
        } else {
            false
        }
    }
}

/// The live counters of the e2e lags that the metrics read: the lags by bucket, the sum of the non-negative ones, the
/// negative ones and the count. A negative lag counts in the first bucket and adds nothing to the sum.
#[derive(Debug, Default)]
pub struct LiveLags {
    pub buckets: [AtomicI64; LAG_BUCKET_BOUNDS_NANOS.len() + 1],
    pub positive_sum_nanos: AtomicI64,
    pub negative: AtomicI64,
    pub count: AtomicI64,
}

impl LiveLags {
    fn add(&self, buckets: &[i64], count: i64, positive_sum: i64, negative: i64) {
        for (live, &added) in self.buckets.iter().zip(buckets) {
            if added != 0 {
                live.fetch_add(added, Ordering::Relaxed);
            }
        }
        self.positive_sum_nanos.fetch_add(positive_sum, Ordering::Relaxed);
        self.negative.fetch_add(negative, Ordering::Relaxed);
        // The count last: a scrape that sees it sees the buckets of the same records or more.
        self.count.fetch_add(count, Ordering::Release);
    }
}

/// What the instances of a consumer share; the listeners write it on the threads of the SDK.
pub struct ConsumerShared {
    stop: Arc<Stop>,
    /// The e2e lags of the current interval.
    pub lags: LagRecorder,
    pub live: LiveLags,
    /// The distinct symbols received by all instances.
    pub symbols_seen: AtomicI64,
    nano: NanoCheck,
    /// The wall-clock time of the first listener call with records, 0 before it: the start of the warm-up.
    first_record_epoch_nanos: AtomicI64,
    /// The monotonic time of the last listener call with records, 0 before the first one.
    last_record_steady_nanos: AtomicI64,
}

impl ConsumerShared {
    pub fn new(stop: Arc<Stop>, nano_check: bool) -> Self {
        ConsumerShared {
            stop,
            lags: LagRecorder::default(),
            live: LiveLags::default(),
            symbols_seen: AtomicI64::new(0),
            nano: NanoCheck::new(nano_check),
            first_record_epoch_nanos: AtomicI64::new(0),
            last_record_steady_nanos: AtomicI64::new(0),
        }
    }

    pub fn first_record_epoch_nanos(&self) -> Option<i64> {
        Some(self.first_record_epoch_nanos.load(Ordering::Relaxed)).filter(|&time| time != 0)
    }

    pub fn last_record_steady_nanos(&self) -> i64 {
        self.last_record_steady_nanos.load(Ordering::Relaxed)
    }

    pub fn nano_missing(&self) -> bool {
        self.nano.is_missing()
    }
}

/// An event of the listener reduced to what the consumer reads.
pub struct Record<'a> {
    /// The time packed with the sequence: `time_sequence` of a Trade, `index` of a TimeAndSale.
    pub packed_time: i64,
    pub nano_part: i32,
    pub symbol: Cow<'a, str>,
}

/// The state of the listener of one instance.
struct Receiver {
    shared: Arc<ConsumerShared>,
    counters: Arc<Counters>,
    /// The symbols of the slice that have not come yet.
    unseen: HashSet<String>,
    lags: Vec<i64>,
    nano_parts: Vec<i32>,
}

impl Receiver {
    fn new(shared: Arc<ConsumerShared>, counters: Arc<Counters>, slice: &[String]) -> Self {
        Receiver {
            shared,
            counters,
            unseen: slice.iter().cloned().collect(),
            lags: Vec::new(),
            nano_parts: Vec::new(),
        }
    }

    /// Takes the events of one listener call; `now_nanos` is the one clock read of the call.
    fn receive<'a>(&mut self, now_nanos: i64, records: impl Iterator<Item = Record<'a>>) {
        let nano_pending = self.shared.nano.is_pending();
        self.lags.clear();
        self.nano_parts.clear();
        let mut buckets = [0_i64; LAG_BUCKET_BOUNDS_NANOS.len() + 1];
        let (mut sum, mut positive_sum, mut negative) = (0_i64, 0_i64, 0_i64);
        for record in records {
            // A negative lag keeps its sign in the sums; the histogram takes it as 0.
            let lag = event_lag_nanos(now_nanos, record.packed_time, record.nano_part);
            self.lags.push(lag);
            sum = sum.wrapping_add(lag);
            buckets[lag_bucket_index(lag)] += 1;
            if lag < 0 {
                negative += 1;
            } else {
                positive_sum = positive_sum.wrapping_add(lag);
            }
            if !self.unseen.is_empty() && self.unseen.remove(&*record.symbol) {
                self.shared.symbols_seen.fetch_add(1, Ordering::Relaxed);
            }
            if nano_pending {
                self.nano_parts.push(record.nano_part);
            }
        }
        let count = self.lags.len() as i64;
        if count == 0 {
            return;
        }
        let shared = &*self.shared;
        let _ = shared
            .first_record_epoch_nanos
            .compare_exchange(0, now_nanos, Ordering::Relaxed, Ordering::Relaxed);
        self.counters.records.fetch_add(count, Ordering::Relaxed);
        self.counters.weighted_nanos.fetch_add(sum, Ordering::Relaxed);
        shared.live.add(&buckets, count, positive_sum, negative);
        shared.lags.record(self.lags.iter().copied());
        shared.last_record_steady_nanos.store(steady_nanos(), Ordering::Relaxed);
        if nano_pending && shared.nano.check(&self.nano_parts) {
            shared.stop.request();
        }
    }
}

/// One feed endpoint with its subscription to a slice of the symbols.
struct Consumer {
    name: String,
    endpoint: DXEndpoint,
    subscription: DXFeedSubscription<Event>,
    counters: Arc<Counters>,
}

impl Consumer {
    /// Creates the endpoint of the role with `dxscheme.nanoTime` on its builder, the subscription, its listener and
    /// then the symbols of the slice. The endpoint connects later, last.
    fn create(name: String, options: &Options, slice: &[String], shared: &Arc<ConsumerShared>) -> Result<Self, String> {
        let endpoint = DXEndpoint::new_builder()
            .with_role(options.role())
            .with_name(&name)
            .with_property(DXEndpoint::DXSCHEME_NANO_TIME_PROPERTY, options.nano_time())
            .build()
            .map_err(|error| format!("cannot create the endpoint {name}: {error}"))?;
        let event_type = match options.event {
            EventKind::Trade => EventType::Trade,
            EventKind::TimeAndSale => EventType::TimeAndSale,
        };
        let subscription = endpoint
            .get_feed()
            .create_subscription_with_types(&[event_type])
            .map_err(|error| format!("cannot subscribe the endpoint {name}: {error}"));
        let subscription = match subscription {
            Ok(subscription) => subscription,
            Err(error) => {
                let _ = endpoint.close_and_await_termination();
                return Err(error);
            }
        };
        let counters = Arc::new(Counters::default());
        let consumer = Consumer {
            name,
            endpoint,
            subscription,
            counters: Arc::clone(&counters),
        };
        consumer.subscribe(options.event, slice, shared)?;
        Ok(consumer)
    }

    fn subscribe(&self, kind: EventKind, slice: &[String], shared: &Arc<ConsumerShared>) -> Result<(), String> {
        let name = &self.name;
        let mut receiver = Receiver::new(Arc::clone(shared), Arc::clone(&self.counters), slice);
        self.subscription
            .add_event_listener(move |events: Events<'_>| {
                // One clock read per call of the listener.
                let now = now_epoch_nanos();
                receiver.receive(
                    now,
                    events.filter_map(|event| match (kind, event) {
                        (EventKind::Trade, EventRef::Trade(trade)) => Some(Record {
                            packed_time: trade.time_sequence(),
                            nano_part: trade.time_nano_part(),
                            symbol: trade.symbol(),
                        }),
                        (EventKind::TimeAndSale, EventRef::TimeAndSale(sale)) => Some(Record {
                            packed_time: sale.index(),
                            nano_part: sale.time_nano_part(),
                            symbol: sale.symbol(),
                        }),
                        _ => None,
                    }),
                );
            })
            .map_err(|error| format!("cannot subscribe the endpoint {name}: {error}"))?;
        // The listener is added before the symbols, so that the first events are not missed.
        if !slice.is_empty() {
            self.subscription
                .add_symbols(slice)
                .map_err(|error| format!("cannot add the symbols of the endpoint {name}: {error}"))?;
        }
        Ok(())
    }

    fn connect(&self, address: &str) -> Result<(), String> {
        let name = &self.name;
        self.endpoint
            .connect(address)
            .map(|_| ())
            .map_err(|error| format!("cannot connect {name} to '{address}': {error}"))
    }

    fn totals(&self) -> InstanceTotals {
        InstanceTotals {
            records: self.counters.records.load(Ordering::Relaxed),
            weighted_nanos: self.counters.weighted_nanos.load(Ordering::Relaxed),
        }
    }

    fn close(&self) {
        let _ = self.subscription.close();
        let _ = self.endpoint.close_and_await_termination();
    }
}

/// How a consumer run ended, and what the summary needs of it.
#[derive(Debug)]
pub struct ConsumerOutcome {
    pub status: Status,
    /// The text after `NetTest: ` on stderr for a failed run; `None` for `ok` and a signal.
    pub error: Option<String>,
    pub reporter: ReporterOutcome,
    pub symbols_seen: i64,
    /// The wall-clock time in nanoseconds since the Unix epoch after the reporter stopped (after the failure when the
    /// endpoints did not start).
    pub end_epoch_nanos: i64,
    /// The statistics of the connectors, read once after the reporter stopped; empty when the endpoints did not start.
    pub connectors: Vec<ConnectorSummary>,
    /// Whether all endpoints started: a run that did not has nothing to report but its failure.
    pub started: bool,
}

impl ConsumerOutcome {
    /// The result for the exit code and the summary file; `start_epoch_nanos` is the time where the run began.
    pub fn into_result(self, start_epoch_nanos: i64) -> RunResult {
        let report = if self.started {
            consumer_report(
                start_epoch_nanos,
                self.end_epoch_nanos,
                self.reporter,
                self.symbols_seen,
                self.connectors,
            )
        } else {
            RunReport::at(start_epoch_nanos, self.end_epoch_nanos)
        };
        RunResult {
            status: self.status,
            error: self.error,
            report,
        }
    }
}

/// The failure that ended the run, the first one that came.
struct Failure(Mutex<Option<(Status, String)>>);

impl Failure {
    fn set(&self, status: Status, message: String, stop: &Stop) {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get_or_insert((status, message));
        stop.request();
    }
}

/// Runs the consumer until the duration, the stop (`signalled` tells a signal from the others), the watchdog or the
/// nanosecond check ends it; the last block is printed and the endpoints are closed before it returns.
pub fn run_consumer(
    options: &Options,
    symbols: &Symbols,
    stop: &Arc<Stop>,
    signalled: &AtomicBool,
    timeouts: WatchdogTimeouts,
) -> ConsumerOutcome {
    let shared = Arc::new(ConsumerShared::new(Arc::clone(stop), options.is_nano_time_on()));
    let failure = Failure(Mutex::new(None));
    let mut consumers: Vec<Consumer> = Vec::with_capacity(options.instances);
    for index in 1..=options.instances {
        let slice = consumer_slice(symbols, index, options.instances);
        let started = Consumer::create(format!("{}.{index}", options.name), options, &slice, &shared).and_then(|c| {
            let connected = c.connect(&options.address);
            // The endpoint that failed to start closes with the others.
            consumers.push(c);
            connected
        });
        if let Err(error) = started {
            for consumer in &consumers {
                consumer.close();
            }
            return ConsumerOutcome {
                status: Status::Error,
                error: Some(error),
                reporter: ReporterOutcome::default(),
                symbols_seen: shared.symbols_seen.load(Ordering::Relaxed),
                end_epoch_nanos: now_epoch_nanos(),
                connectors: Vec::new(),
                started: false,
            };
        }
    }
    let reporter_stop = Stop::default();
    let reporter_outcome = std::thread::scope(|scope| {
        let consumers = &consumers;
        let shared = &shared;
        let reporter = scope.spawn(|| {
            run_reporter(
                ReporterSettings {
                    name: &options.name,
                    side: Side::Consumer,
                    period_millis: options.period_millis,
                    warmup_millis: options.warmup_millis,
                },
                system_clock(&reporter_stop),
                || StatisticsSample {
                    instances: consumers.iter().map(Consumer::totals).collect(),
                    connections: consumers.iter().map(|c| connections_of(&c.endpoint)).sum(),
                    // A failed read prints n/a values, as in the C tool.
                    runtime: Some(
                        system::get_runtime_statistics()
                            .map(|statistics| RuntimeSample::from_statistics(&statistics))
                            .unwrap_or_default(),
                    ),
                },
                || IntervalTaken {
                    warmup_start_epoch_nanos: shared.first_record_epoch_nanos(),
                    lags: Some(shared.lags.take()),
                    ..IntervalTaken::default()
                },
                crate::console::print,
            )
        });
        let deadline = options
            .duration_millis
            .map(|millis| Instant::now() + Duration::from_millis(millis as u64));
        let mut watchdog = Watchdog::new(timeouts);
        while !stop.is_stopped() {
            let mut wait = WATCHDOG_PERIOD;
            if let Some(deadline) = deadline {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    break;
                }
                wait = wait.min(left);
            }
            if stop.wait_for(wait) {
                break;
            }
            let connections: i64 = consumers.iter().map(|c| connections_of(&c.endpoint)).sum();
            if let Some(message) = watchdog.check(steady_nanos(), connections, shared.last_record_steady_nanos()) {
                failure.set(Status::NoData, message, stop);
            }
        }
        stop.request();
        reporter_stop.request();
        reporter.join()
    });
    // The report is taken before the endpoints close: a closed connector reads 0 for its records.
    let end_epoch_nanos = now_epoch_nanos();
    let connectors = connector_summaries(consumers.iter().map(|consumer| &consumer.endpoint));
    for consumer in &consumers {
        consumer.close();
    }
    let reporter = match reporter_outcome {
        Ok(outcome) => outcome,
        Err(_) => {
            failure.set(Status::Error, "the reporter thread panicked".to_owned(), stop);
            ReporterOutcome::default()
        }
    };
    if shared.nano_missing() {
        failure.set(
            Status::NanoMissing,
            format!(
                "the first {NANO_CHECK_EVENTS} events have the nanosecond part 0: a hop of the path lacks \
                 dxscheme.nanoTime=true (run with -Ddxscheme.nanoTime=false to receive without it)"
            ),
            stop,
        );
    }
    let failed = failure.0.into_inner().unwrap_or_else(|poisoned| poisoned.into_inner());
    let (status, error) = match failed {
        Some((status, message)) if status == Status::Error => (status, Some(message)),
        Some((status, message)) => (status, Some(format!("{}: {message}", status.name()))),
        None if signalled.load(Ordering::SeqCst) => (Status::StoppedBySignal, None),
        None => (Status::Ok, None),
    };
    ConsumerOutcome {
        status,
        error,
        reporter,
        symbols_seen: shared.symbols_seen.load(Ordering::Relaxed),
        end_epoch_nanos,
        connectors,
        started: true,
    }
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;
    use std::sync::atomic::AtomicBool;

    use dxfeed_graal::event::Trade;
    use dxfeed_graal::{DXEndpoint, Role};

    use super::super::cli::{Options, parse};
    use super::super::events::pack_time_sequence;
    use super::super::symbols::generate_symbols;
    use super::*;

    const EPOCH: i64 = 1_700_000_000_123_456_789;
    const SECOND: i64 = 1_000_000_000;

    fn symbol_list(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    fn record(packed_time: i64, nano_part: i32, symbol: &str) -> Record<'_> {
        Record {
            packed_time,
            nano_part,
            symbol: Cow::Borrowed(symbol),
        }
    }

    fn receiver(slice: &[String], nano_check: bool) -> (Receiver, Arc<ConsumerShared>, Arc<Stop>) {
        let stop = Arc::new(Stop::default());
        let shared = Arc::new(ConsumerShared::new(Arc::clone(&stop), nano_check));
        let receiver = Receiver::new(Arc::clone(&shared), Arc::new(Counters::default()), slice);
        (receiver, shared, stop)
    }

    #[test]
    fn five_symbols_over_two_instances_are_sliced_by_index() {
        let symbols = symbol_list(&["A", "B", "C", "D", "E"]);
        assert_eq!(consumer_slice(&symbols, 1, 2), symbol_list(&["A", "C", "E"]));
        assert_eq!(consumer_slice(&symbols, 2, 2), symbol_list(&["B", "D"]));
    }

    #[test]
    fn the_lag_is_now_minus_the_packed_time_and_the_nanosecond_part() {
        let packed = pack_time_sequence(EPOCH, 7);
        // The milliseconds of the packed time plus the part are the sending time exactly.
        assert_eq!(event_lag_nanos(EPOCH + 1500, packed, 456_789), 1500);
        // Without the part the lag grows by it.
        assert_eq!(event_lag_nanos(EPOCH + 1500, packed, 0), 1500 + 456_789);
        // A time in the future keeps its sign.
        assert_eq!(event_lag_nanos(EPOCH - 2000, packed, 456_789), -2000);
        assert_eq!(event_lag_nanos(EPOCH - 2000, packed, 0), -2000 + 456_789);
    }

    #[test]
    fn the_bucket_of_a_lag_is_the_first_bound_not_below_it() {
        assert_eq!(lag_bucket_index(-5), 0);
        assert_eq!(lag_bucket_index(50_000), 0);
        assert_eq!(lag_bucket_index(50_001), 1);
        assert_eq!(lag_bucket_index(1_000_000_000), 12);
        assert_eq!(lag_bucket_index(1_000_000_001), 13);
    }

    #[test]
    fn the_receiver_counts_records_signs_and_distinct_symbols() {
        let slice = symbol_list(&["A", "B", "C"]);
        let (mut receiver, shared, _stop) = receiver(&slice, false);
        let packed = pack_time_sequence(EPOCH, 0);
        receiver.receive(
            EPOCH + 3000,
            [
                record(packed, 456_789, "A"),
                record(packed, 456_789, "A"),
                record(packed, 456_789, "Z"),
            ]
            .into_iter(),
        );
        // A call without events changes nothing.
        receiver.receive(EPOCH, std::iter::empty());
        // The clock behind the sender: a negative lag.
        receiver.receive(EPOCH - 1000, [record(packed, 456_789, "B")].into_iter());
        assert_eq!(shared.symbols_seen.load(Ordering::Relaxed), 2);
        assert_eq!(shared.first_record_epoch_nanos(), Some(EPOCH + 3000));
        assert_ne!(shared.last_record_steady_nanos(), 0);
        assert_eq!(receiver.counters.records.load(Ordering::Relaxed), 4);
        assert_eq!(
            receiver.counters.weighted_nanos.load(Ordering::Relaxed),
            3 * 3000 - 1000
        );
        let lags = shared.lags.take();
        assert_eq!(
            (lags.stats.count, lags.stats.negative, lags.stats.sum, lags.stats.max),
            (4, 1, 8000, 3000)
        );
        // The histogram takes the negative lag as 0 and the others at their value.
        assert_eq!(lags.histogram.count(), 4);
        assert_eq!(lags.histogram.value_at_percentile(0.0), 0);
        assert_eq!(shared.live.count.load(Ordering::Relaxed), 4);
        assert_eq!(shared.live.negative.load(Ordering::Relaxed), 1);
        assert_eq!(shared.live.positive_sum_nanos.load(Ordering::Relaxed), 9000);
        assert_eq!(shared.live.buckets[0].load(Ordering::Relaxed), 4);
    }

    #[test]
    fn the_watchdog_waits_for_the_connection_and_for_the_records() {
        let mut watchdog = Watchdog::new(WatchdogTimeouts::default());
        let minute = 60 * SECOND;
        // No verdict while nothing is connected, however long it takes.
        assert_eq!(watchdog.check(1, 0, 0), None);
        assert_eq!(watchdog.check(10 * minute, 0, 0), None);
        // Connected at 10 minutes, no record: 60 s after the connection.
        assert_eq!(watchdog.check(10 * minute, 2, 0), None);
        assert_eq!(watchdog.check(10 * minute + 59 * SECOND, 2, 0), None);
        let message = watchdog.check(10 * minute + 60 * SECOND, 2, 0).unwrap();
        assert_eq!(
            message,
            "no records for 60000 ms after the connection, with 2 open connections"
        );
        // A disconnect forgets the connection time.
        assert_eq!(watchdog.check(20 * minute, 0, 0), None);
        assert_eq!(watchdog.check(20 * minute + 10 * SECOND, 1, 0), None);
        assert_eq!(watchdog.check(20 * minute + 69 * SECOND, 1, 0), None);
        // A record at 21 minutes: 30 s after it, not 60 s after the connection.
        let record = 21 * minute;
        assert_eq!(watchdog.check(record + 29 * SECOND, 1, record), None);
        let message = watchdog.check(record + 30 * SECOND, 1, record).unwrap();
        assert_eq!(
            message,
            "no records for 30000 ms after the last record, with 1 open connections"
        );
    }

    #[test]
    fn a_record_that_came_before_the_connection_counts_from_the_connection() {
        let mut watchdog = Watchdog::new(WatchdogTimeouts::default());
        // The last record is old, but the connection is new: the 30 s run from the connection.
        assert_eq!(watchdog.check(100 * SECOND, 1, 10 * SECOND), None);
        assert_eq!(watchdog.check(129 * SECOND, 1, 10 * SECOND), None);
        assert!(watchdog.check(130 * SECOND, 1, 10 * SECOND).is_some());
    }

    #[test]
    fn the_variable_shortens_the_watchdog_and_a_bad_value_keeps_the_defaults() {
        let short = WatchdogTimeouts::from_variable(Some("1500, 2500"));
        assert_eq!(short.after_record, Duration::from_millis(1500));
        assert_eq!(short.after_connect, Duration::from_millis(2500));
        let default = WatchdogTimeouts::default();
        assert_eq!(default.after_record, Duration::from_secs(30));
        assert_eq!(default.after_connect, Duration::from_secs(60));
        for value in [None, Some(""), Some("1500"), Some("a,b"), Some("1,"), Some("-1,2")] {
            assert_eq!(WatchdogTimeouts::from_variable(value), default, "{value:?}");
        }
    }

    #[test]
    fn a_thousand_zero_parts_stop_the_run() {
        let slice = symbol_list(&["A"]);
        let (mut receiver, shared, stop) = receiver(&slice, true);
        let packed = pack_time_sequence(EPOCH, 0);
        // 999 events, then the thousandth in a later call.
        receiver.receive(EPOCH, (0..999).map(|_| record(packed, 0, "A")));
        assert!(!stop.is_stopped() && !shared.nano_missing());
        receiver.receive(EPOCH, std::iter::once(record(packed, 0, "A")));
        assert!(stop.is_stopped() && shared.nano_missing());
    }

    #[test]
    fn one_nonzero_part_clears_the_check() {
        let slice = symbol_list(&["A"]);
        let (mut receiver, shared, stop) = receiver(&slice, true);
        let packed = pack_time_sequence(EPOCH, 0);
        receiver.receive(EPOCH, (0..999).map(|_| record(packed, 0, "A")));
        receiver.receive(EPOCH, std::iter::once(record(packed, 1, "A")));
        assert!(!shared.nano.is_pending());
        // Later zero parts are not looked at anymore.
        receiver.receive(EPOCH, (0..5000).map(|_| record(packed, 0, "A")));
        assert!(!stop.is_stopped() && !shared.nano_missing());
    }

    #[test]
    fn the_check_counts_only_the_first_thousand_events_of_one_big_call() {
        let check = NanoCheck::new(true);
        let mut parts = vec![0; 1000];
        parts.push(5);
        assert!(check.check(&parts));
        assert!(check.is_missing());
        // Decided: a later call changes nothing.
        assert!(!check.check(&[5]));
    }

    #[test]
    fn a_disabled_check_never_stops() {
        let slice = symbol_list(&["A"]);
        let (mut receiver, shared, stop) = receiver(&slice, false);
        let packed = pack_time_sequence(EPOCH, 0);
        assert!(!shared.nano.is_pending());
        receiver.receive(EPOCH, (0..5000).map(|_| record(packed, 0, "A")));
        assert!(!stop.is_stopped() && !shared.nano_missing());
    }

    #[test]
    fn the_statuses_have_the_names_and_the_exit_codes_of_the_specification() {
        let table = [
            (Status::Ok, "ok", 0),
            (Status::StoppedBySignal, "stopped_by_signal", 0),
            (Status::NoData, "no_data", 1),
            (Status::NanoMissing, "nano_missing", 1),
            (Status::Error, "error", 1),
        ];
        for (status, name, code) in table {
            assert_eq!((status.name(), status.exit_code()), (name, code));
        }
    }

    // The tests below run the consumer of the tool against a publisher endpoint of the test process on loopback.

    fn free_port() -> u16 {
        TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
    }

    fn options(args: &[&str]) -> Options {
        parse(&args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>()).unwrap()
    }

    /// A publisher that publishes a trade of each symbol every few milliseconds until it is dropped.
    struct TestPublisher {
        endpoint: DXEndpoint,
        done: Arc<AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl TestPublisher {
        fn start(port: u16, nano_time: &str, symbols: Vec<String>) -> Self {
            let endpoint = DXEndpoint::new_builder()
                .with_role(Role::Publisher)
                .with_property(DXEndpoint::DXSCHEME_NANO_TIME_PROPERTY, nano_time)
                .build()
                .unwrap();
            endpoint.connect(&format!(":{port}")).unwrap();
            let publisher = endpoint.get_publisher();
            let done = Arc::new(AtomicBool::new(false));
            let flag = Arc::clone(&done);
            let thread = std::thread::spawn(move || {
                let mut sequence = 0_u32;
                while !flag.load(Ordering::Relaxed) {
                    let now = now_epoch_nanos();
                    let events: Vec<Event> = symbols
                        .iter()
                        .map(|symbol| {
                            sequence += 1;
                            let mut trade = Trade::new(symbol.clone());
                            trade.time_sequence = pack_time_sequence(now, sequence);
                            // Always at least 1, so that the part is not 0 where the property carries it.
                            trade.time_nano_part = (now % 1_000_000) as i32 | 1;
                            Event::Trade(trade)
                        })
                        .collect();
                    publisher.publish_events(&events).unwrap();
                    std::thread::sleep(Duration::from_millis(2));
                }
            });
            TestPublisher {
                endpoint,
                done,
                thread: Some(thread),
            }
        }
    }

    impl Drop for TestPublisher {
        fn drop(&mut self) {
            self.done.store(true, Ordering::Relaxed);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
            let _ = self.endpoint.close_and_await_termination();
        }
    }

    /// Runs the consumer of the tool with the arguments against a publisher with the property value.
    fn run_against_publisher(publisher_nano_time: &str, extra: &[&str]) -> (ConsumerOutcome, Duration) {
        let port = free_port();
        let address = format!("127.0.0.1:{port}");
        let mut args = vec!["c", &address, "-S", "5", "-s", "1s"];
        args.extend_from_slice(extra);
        let options = options(&args);
        let symbols: Symbols = Arc::new(generate_symbols(5));
        let _publisher = TestPublisher::start(port, publisher_nano_time, symbols.to_vec());
        let stop = Arc::new(Stop::default());
        let started = Instant::now();
        let outcome = run_consumer(
            &options,
            &symbols,
            &stop,
            &AtomicBool::new(false),
            WatchdogTimeouts::default(),
        );
        (outcome, started.elapsed())
    }

    #[test]
    fn a_publisher_with_the_builder_property_feeds_two_consumer_instances() {
        let (outcome, _) = run_against_publisher("true", &["-C", "2", "--duration", "4s"]);
        assert_eq!(outcome.status, Status::Ok, "{outcome:?}");
        assert_eq!(outcome.error, None);
        // The two slices together hold the five symbols, and every one of them came.
        assert_eq!(outcome.symbols_seen, 5, "{outcome:?}");
        let records: i64 = outcome.reporter.intervals.iter().map(|interval| interval.records).sum();
        assert!(records > 0, "{outcome:?}");
        // Every record gives a lag. As in the C tool, the reporter reads the record counters and then takes the lags,
        // while the listeners still run, so the last interval may hold a few lags more or less than records.
        let lags = outcome.reporter.e2e.stats.count;
        assert!(
            (lags - records).abs() * 100 <= records,
            "{lags} lags, {records} records"
        );
    }

    #[test]
    fn a_publisher_without_the_property_stops_the_consumer_with_nano_missing() {
        let (outcome, elapsed) = run_against_publisher("false", &["--duration", "40s"]);
        assert_eq!(outcome.status, Status::NanoMissing, "{outcome:?}");
        assert!(
            outcome.error.as_deref().is_some_and(
                |error| error.starts_with("nano_missing: the first 1000 events have the nanosecond part 0")
            ),
            "{outcome:?}"
        );
        assert!(elapsed < Duration::from_secs(30), "{elapsed:?}");
    }

    #[test]
    fn with_the_property_off_on_both_sides_the_consumer_runs_to_the_end() {
        let (outcome, elapsed) = run_against_publisher("false", &["--duration", "3s", "-Ddxscheme.nanoTime=false"]);
        assert_eq!(outcome.status, Status::Ok, "{outcome:?}");
        assert_eq!(outcome.symbols_seen, 5, "{outcome:?}");
        assert!(elapsed >= Duration::from_secs(3), "{elapsed:?}");
    }

    #[test]
    fn a_consumer_that_cannot_connect_has_no_verdict_and_ends_at_the_duration() {
        // Nothing listens on the port: the connection stays connecting and the watchdog stays silent.
        let port = free_port();
        let address = format!("127.0.0.1:{port}");
        let options = options(&["c", &address, "-S", "5", "--duration", "2s"]);
        let symbols: Symbols = Arc::new(generate_symbols(5));
        let stop = Arc::new(Stop::default());
        let timeouts = WatchdogTimeouts {
            after_record: Duration::from_millis(100),
            after_connect: Duration::from_millis(100),
        };
        let outcome = run_consumer(&options, &symbols, &stop, &AtomicBool::new(false), timeouts);
        assert_eq!(outcome.status, Status::Ok, "{outcome:?}");
        assert_eq!(outcome.symbols_seen, 0);
    }

    #[test]
    fn a_stop_by_signal_is_reported() {
        let port = free_port();
        let address = format!("127.0.0.1:{port}");
        let options = options(&["c", &address, "-S", "5"]);
        let symbols: Symbols = Arc::new(generate_symbols(5));
        let stop = Arc::new(Stop::default());
        let signalled = AtomicBool::new(false);
        let outcome = std::thread::scope(|scope| {
            scope.spawn(|| {
                std::thread::sleep(Duration::from_millis(1500));
                signalled.store(true, Ordering::SeqCst);
                stop.request();
            });
            run_consumer(&options, &symbols, &stop, &signalled, WatchdogTimeouts::default())
        });
        assert_eq!(outcome.status, Status::StoppedBySignal, "{outcome:?}");
        assert_eq!(outcome.error, None);
        assert_eq!(outcome.status.exit_code(), 0);
    }
}
