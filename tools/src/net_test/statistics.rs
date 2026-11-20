//! The statistics of NetTest: the lines of both sides (the producer and the consumer block, per-instance lines and the
//! runtime line of the isolate), the reporter that prints them on the ticks aligned to the wall clock, and the interval
//! records that the reporter keeps for the summary.

use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use dxfeed_graal::system::RuntimeStatistics;

use super::histogram::Histogram;
use super::limiter::Stop;

const NANOS_PER_SECOND: i64 = 1_000_000_000;
const NANOS_PER_MICRO: i64 = 1_000;
const NANOS_PER_MILLI: i64 = 1_000_000;
const BYTES_PER_MB: i64 = 1024 * 1024;
/// The shortest last block that the reporter prints at the stop.
const MIN_LAST_BLOCK_NANOS: i64 = NANOS_PER_SECOND;
const ABSENT_TEXT: &str = "n/a";

/// The side of the statistics: the label of the summary line and the meaning of its lag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// `p`, `NetTestProducerSide`: the lag is the time to fill and publish a batch, weighted by its records.
    Producer,
    /// `c`, `NetTestConsumerSide`: the lag is the mean e2e lag of the records, which may be negative.
    Consumer,
}

impl Side {
    fn label(self) -> &'static str {
        match self {
            Side::Producer => "NetTestProducerSide",
            Side::Consumer => "NetTestConsumerSide",
        }
    }
}

/// The cumulative counters of one instance.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InstanceTotals {
    pub records: i64,
    /// On `p`, the sum of records * (the time to build and publish the batch of those records) in nanoseconds; on `c`,
    /// the sum of the e2e lags of the records in nanoseconds, with their signs.
    pub weighted_nanos: i64,
}

/// The runtime values of the isolate that the runtime line prints: the heap now and the cumulative GC and CPU
/// counters; `None` when the isolate does not give one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RuntimeSample {
    pub heap_used_bytes: Option<i64>,
    pub heap_committed_bytes: Option<i64>,
    pub young_gc_count: Option<i64>,
    pub young_gc_time_ms: Option<i64>,
    pub full_gc_count: Option<i64>,
    pub full_gc_time_ms: Option<i64>,
    pub process_cpu_time_ns: Option<i64>,
}

impl RuntimeSample {
    /// The values of the statistics.
    pub fn from_statistics(statistics: &RuntimeStatistics) -> Self {
        let count = |value: Option<u64>| value.and_then(|value| i64::try_from(value).ok());
        let millis = |value: Option<Duration>| value.and_then(|value| i64::try_from(value.as_millis()).ok());
        RuntimeSample {
            heap_used_bytes: count(statistics.heap_used_bytes),
            heap_committed_bytes: count(statistics.heap_committed_bytes),
            young_gc_count: count(statistics.young_gc_count),
            young_gc_time_ms: millis(statistics.young_gc_time),
            full_gc_count: count(statistics.full_gc_count),
            full_gc_time_ms: millis(statistics.full_gc_time),
            process_cpu_time_ns: statistics
                .process_cpu_time
                .and_then(|value| i64::try_from(value.as_nanos()).ok()),
        }
    }
}

/// The cumulative counters of all instances, the connections open now and the runtime values; without them the
/// reporter prints no runtime line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StatisticsSample {
    pub instances: Vec<InstanceTotals>,
    pub connections: i64,
    pub runtime: Option<RuntimeSample>,
}

/// The e2e lags of a period with their signs: the count, the negative ones among them, the sum and the largest; the
/// largest is valid when the count is not 0.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LagStats {
    pub count: i64,
    pub negative: i64,
    pub sum: i64,
    pub max: i64,
}

impl LagStats {
    fn add_lag(&mut self, lag: i64) {
        self.max = if self.count == 0 { lag } else { self.max.max(lag) };
        self.sum += lag;
        self.negative += i64::from(lag < 0);
        self.count += 1;
    }

    fn add(&mut self, other: &LagStats) {
        if other.count == 0 {
            return;
        }
        self.max = if self.count == 0 {
            other.max
        } else {
            self.max.max(other.max)
        };
        self.sum += other.sum;
        self.negative += other.negative;
        self.count += other.count;
    }
}

/// The e2e lags of one interval of all consumer instances: their statistics and their histogram, where a negative lag
/// enters as 0.
#[derive(Debug, Clone, Default)]
pub struct LagInterval {
    pub stats: LagStats,
    pub histogram: Histogram,
}

/// The e2e lags of the current interval: the listeners record them, the reporter takes them at the end of the
/// interval.
#[derive(Debug, Default)]
pub struct LagRecorder {
    current: Mutex<LagInterval>,
}

impl LagRecorder {
    /// Records the lags of one listener call, in nanoseconds with their signs.
    pub fn record(&self, lags: impl IntoIterator<Item = i64>) {
        let mut current = self.current.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        for lag in lags {
            current.histogram.record(lag);
            current.stats.add_lag(lag);
        }
    }

    /// Takes the lags of the interval and starts the next one.
    pub fn take(&self) -> LagInterval {
        // The new histogram is allocated outside of the lock: the listeners wait for it.
        let next = LagInterval::default();
        std::mem::replace(
            &mut *self.current.lock().unwrap_or_else(|poisoned| poisoned.into_inner()),
            next,
        )
    }
}

/// The number with `,` grouping of the thousands, as `NumberFormat.getIntegerInstance(Locale.US)` prints it.
pub fn group_digits(value: i64) -> String {
    let digits = value.unsigned_abs().to_string();
    let mut result = String::with_capacity(digits.len() + digits.len() / 3 + 1);
    if value < 0 {
        result.push('-');
    }
    for (index, digit) in digits.chars().enumerate() {
        if index != 0 && (digits.len() - index).is_multiple_of(3) {
            result.push(',');
        }
        result.push(digit);
    }
    result
}

/// Records per second over the period, integer division; 0 for an empty period.
pub fn records_per_second(records: i64, period_nanos: i64) -> i64 {
    if period_nanos <= 0 || records <= 0 {
        return 0;
    }
    (i128::from(records) * i128::from(NANOS_PER_SECOND) / i128::from(period_nanos)) as i64
}

/// The record-weighted mean time to build and publish a batch in microseconds, integer division; 0 without records.
pub fn lag_micros(records: i64, weighted_nanos: i64) -> i64 {
    if records <= 0 || weighted_nanos <= 0 {
        return 0;
    }
    weighted_nanos / records / NANOS_PER_MICRO
}

/// The mean e2e lag in microseconds, `lag_sum_nanos / records / 1000` divided toward zero, so it may be negative; 0
/// without records.
pub fn mean_lag_micros(records: i64, lag_sum_nanos: i64) -> i64 {
    if records <= 0 {
        return 0;
    }
    lag_sum_nanos / records / NANOS_PER_MICRO
}

/// The text of a block without the line break at the end: the summary line, then one line per instance, also for one.
pub fn format_statistics(
    side: Side,
    name: &str,
    deltas: &[InstanceTotals],
    period_nanos: i64,
    connections: i64,
) -> String {
    let lag = |records: i64, nanos: i64| match side {
        Side::Producer => lag_micros(records, nanos),
        Side::Consumer => mean_lag_micros(records, nanos),
    };
    let records: i64 = deltas.iter().map(|delta| delta.records).sum();
    let weighted_nanos: i64 = deltas.iter().map(|delta| delta.weighted_nanos).sum();
    let mut text = format!(
        "{{*{}-{name}*}} data {} rps, lag {} us, instances {}, connections {connections}",
        side.label(),
        group_digits(records_per_second(records, period_nanos)),
        group_digits(lag(records, weighted_nanos)),
        deltas.len()
    );
    for (index, delta) in deltas.iter().enumerate() {
        text.push_str(&format!(
            "\n    {name}.{} data {} rps, lag {} us",
            index + 1,
            group_digits(records_per_second(delta.records, period_nanos)),
            group_digits(lag(delta.records, delta.weighted_nanos))
        ));
    }
    text
}

/// The deltas of the instances between two samples; an instance that the previous sample lacks counts from 0.
fn deltas(previous: &StatisticsSample, current: &StatisticsSample) -> Vec<InstanceTotals> {
    current
        .instances
        .iter()
        .enumerate()
        .map(|(index, current)| match previous.instances.get(index) {
            Some(previous) => InstanceTotals {
                records: current.records - previous.records,
                weighted_nanos: current.weighted_nanos - previous.weighted_nanos,
            },
            None => *current,
        })
        .collect()
}

fn megabytes(bytes: Option<i64>) -> String {
    bytes.map_or_else(
        || ABSENT_TEXT.to_owned(),
        |bytes| format!("{} MB", group_digits(bytes / BYTES_PER_MB)),
    )
}

/// The growth of a cumulative counter; absent when either value is absent or the counter went back.
fn growth(previous: Option<i64>, current: Option<i64>) -> Option<i64> {
    let (previous, current) = (previous?, current?);
    (current >= previous).then(|| current - previous)
}

/// The sum of two growths; absent when either is absent.
fn sum(first: Option<i64>, second: Option<i64>) -> Option<i64> {
    Some(first? + second?)
}

/// The runtime line of the period between two samples, without the line break: the heap of the current sample in MB
/// (MiB, rounded down), the young plus full collections of the period with their time, and the process CPU time of
/// the period as a share of its length (above 100% on several cores).
pub fn format_runtime(name: &str, previous: &RuntimeSample, current: &RuntimeSample, period_nanos: i64) -> String {
    let gc_count = sum(
        growth(previous.young_gc_count, current.young_gc_count),
        growth(previous.full_gc_count, current.full_gc_count),
    );
    let gc_time_ms = sum(
        growth(previous.young_gc_time_ms, current.young_gc_time_ms),
        growth(previous.full_gc_time_ms, current.full_gc_time_ms),
    );
    let cpu_nanos = growth(previous.process_cpu_time_ns, current.process_cpu_time_ns);
    let gc = match (gc_count, gc_time_ms) {
        (Some(count), Some(time)) => format!("{} ({} ms)", group_digits(count), group_digits(time)),
        _ => ABSENT_TEXT.to_owned(),
    };
    let cpu = match cpu_nanos {
        Some(cpu) if period_nanos > 0 => format!("{}%", group_digits(cpu * 100 / period_nanos)),
        _ => ABSENT_TEXT.to_owned(),
    };
    format!(
        "{{*NetTestRuntime-{name}*}} heap {} of {}, gc {gc}, cpu {cpu}",
        megabytes(current.heap_used_bytes),
        megabytes(current.heap_committed_bytes)
    )
}

/// The record of one interval of the reporter, printed or not: what the summary file keeps per interval. The bounds
/// are on the wall clock. The intervals of a run cover it without a gap, and their records add up to the records of
/// the run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntervalRecord {
    pub start_epoch_nanos: i64,
    pub end_epoch_nanos: i64,
    /// The interval belongs to the warm-up: the phase `warmup` of the summary, `measure` otherwise.
    pub warmup: bool,
    pub records: i64,
    /// Records per second over the real length of the interval.
    pub rate: i64,
    /// `c`, with records: the mean (divided toward zero), the median and the 99th percentile of the e2e lag of the
    /// interval, and its largest lag, in nanoseconds.
    pub lag_mean_ns: Option<i64>,
    pub lag_p50_ns: Option<i64>,
    pub lag_p99_ns: Option<i64>,
    pub lag_max_ns: Option<i64>,
    /// `p`, with records: the fill and publishing call per event, in nanoseconds.
    pub publish_ns_per_event: Option<i64>,
    /// `p` with a schedule: the largest slip of the interval and the slip at its end, in nanoseconds.
    pub slip_max_ns: Option<i64>,
    pub slip_end_ns: Option<i64>,
    /// The connections at the end of the interval.
    pub connections: i64,
}

impl IntervalRecord {
    pub fn start_epoch_ms(&self) -> i64 {
        self.start_epoch_nanos / NANOS_PER_MILLI
    }

    pub fn end_epoch_ms(&self) -> i64 {
        self.end_epoch_nanos / NANOS_PER_MILLI
    }
}

/// What the reporter takes from the live counters once at the end of each interval, besides the cumulative sample:
/// the values that reset or that are not totals of the instances.
#[derive(Debug, Clone, Default)]
pub struct IntervalTaken {
    /// The wall-clock time in nanoseconds since the Unix epoch where the warm-up starts: the first grant on `p`, the
    /// first record on `c`; `None` before it, so that every interval before it is warm-up.
    pub warmup_start_epoch_nanos: Option<i64>,
    /// `p`: the cumulative nanoseconds of the fills and publishing calls of all generators.
    pub publish_nanos_total: i64,
    /// `p` with a schedule: the largest slip since the previous interval and the slip of the last batch; `None`
    /// without a schedule.
    pub slip_max_nanos: Option<i64>,
    pub slip_end_nanos: Option<i64>,
    /// `c`: the lags of the interval, taken from the recorder.
    pub lags: Option<LagInterval>,
}

/// The e2e lags of the measured intervals (after the warm-up) of the run: the data of `e2e_lag` of the summary.
#[derive(Debug, Clone, Default)]
pub struct E2eTotals {
    pub stats: LagStats,
    /// The histogram of the lags; its percentiles are the percentiles of the summary.
    pub histogram: Histogram,
}

/// A runtime sample with the wall-clock time where it was read, in nanoseconds since the Unix epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimedRuntime {
    pub sample: RuntimeSample,
    pub epoch_nanos: i64,
}

/// What the reporter leaves for the summary after the stop.
#[derive(Debug, Clone, Default)]
pub struct ReporterOutcome {
    /// Every interval from the start of the reporter to its stop, in order.
    pub intervals: Vec<IntervalRecord>,
    /// `c`: the totals of the measured intervals; empty on `p`.
    pub e2e: E2eTotals,
    /// The runtime values at the start of the reporter and at its stop, when the isolate gave them.
    pub first_runtime: Option<TimedRuntime>,
    pub last_runtime: Option<TimedRuntime>,
}

/// What the reporter needs to know about the run.
#[derive(Debug, Clone, Copy)]
pub struct ReporterSettings<'a> {
    pub name: &'a str,
    pub side: Side,
    pub period_millis: i64,
    /// `--warmup`; without it no interval is warm-up.
    pub warmup_millis: Option<i64>,
}

/// An interval belongs to the warm-up when the warm-up has not started (`warmup_start` is `None`, or 0) or the interval
/// started before the end of the warm-up: one that overlaps the end counts as warm-up.
pub fn is_warmup_interval(warmup_nanos: i64, warmup_start_epoch_nanos: Option<i64>, interval_start_nanos: i64) -> bool {
    match warmup_start_epoch_nanos {
        None | Some(0) => true,
        Some(start) => interval_start_nanos < start.saturating_add(warmup_nanos),
    }
}

/// The clock of the reporter, replaceable in tests.
pub struct ReporterClock<N, S> {
    /// The wall clock in nanoseconds since the Unix epoch.
    pub now_nanos: N,
    /// Sleeps until the wall-clock time; false when the reporter must stop.
    pub sleep_until_nanos: S,
}

/// The system clock with a sleep that ends at the stop.
pub fn system_clock(stop: &Stop) -> ReporterClock<impl Fn() -> i64, impl FnMut(i64) -> bool + '_> {
    ReporterClock {
        now_nanos: super::events::now_epoch_nanos,
        sleep_until_nanos: move |nanos: i64| {
            let time = SystemTime::UNIX_EPOCH + Duration::from_nanos(u64::try_from(nanos).unwrap_or(0));
            !stop.wait_until(time)
        },
    }
}

/// Collects the records of the intervals.
struct Collector<'a> {
    settings: ReporterSettings<'a>,
    outcome: ReporterOutcome,
    /// The sample at the start of the interval; the first interval starts from the zero counters.
    interval_start: StatisticsSample,
    publish_nanos_total: i64,
}

impl Collector<'_> {
    fn push(&mut self, current: &StatisticsSample, start_nanos: i64, end_nanos: i64, taken: IntervalTaken) {
        let warmup = self.settings.warmup_millis.is_some_and(|millis| {
            is_warmup_interval(
                millis.saturating_mul(NANOS_PER_MILLI),
                taken.warmup_start_epoch_nanos,
                start_nanos,
            )
        });
        let length = end_nanos - start_nanos;
        let records: i64 = deltas(&self.interval_start, current)
            .iter()
            .map(|delta| delta.records)
            .sum();
        let mut record = IntervalRecord {
            start_epoch_nanos: start_nanos,
            end_epoch_nanos: end_nanos,
            warmup,
            records,
            rate: records_per_second(records, length),
            lag_mean_ns: None,
            lag_p50_ns: None,
            lag_p99_ns: None,
            lag_max_ns: None,
            publish_ns_per_event: None,
            slip_max_ns: None,
            slip_end_ns: None,
            connections: current.connections,
        };
        match self.settings.side {
            Side::Producer => {
                if records > 0 {
                    record.publish_ns_per_event =
                        Some((taken.publish_nanos_total - self.publish_nanos_total) / records);
                }
                self.publish_nanos_total = taken.publish_nanos_total;
                record.slip_max_ns = taken.slip_max_nanos;
                record.slip_end_ns = taken.slip_end_nanos;
            }
            Side::Consumer => {
                if let Some(lags) = taken.lags.filter(|lags| lags.stats.count > 0) {
                    // Rust divides toward zero, as the specification wants for a negative sum.
                    record.lag_mean_ns = Some(lags.stats.sum / lags.stats.count);
                    record.lag_p50_ns = Some(lags.histogram.value_at_percentile(50.0));
                    record.lag_p99_ns = Some(lags.histogram.value_at_percentile(99.0));
                    record.lag_max_ns = Some(lags.stats.max);
                    if !warmup {
                        self.outcome.e2e.histogram.add(&lags.histogram);
                        self.outcome.e2e.stats.add(&lags.stats);
                    }
                }
            }
        }
        self.outcome.intervals.push(record);
        self.interval_start = current.clone();
    }
}

/// Runs until the clock stops it. Sleeps to the next wall-clock multiple of the period and prints the deltas against
/// the previous tick; the first tick only starts the count and prints nothing. At the stop it prints the partial
/// period since the last tick (or since the start) at its real length, unless it is shorter than one second. When both
/// samples have runtime values, the runtime line follows the block in the same text.
///
/// Every interval is recorded, printed or not: from the start to the first tick, between ticks and from the last tick
/// to the stop, so the records cover the whole run. `take_interval` is called once per interval, after the sample of
/// its end.
pub fn run_reporter<N: Fn() -> i64, S: FnMut(i64) -> bool>(
    settings: ReporterSettings<'_>,
    mut clock: ReporterClock<N, S>,
    mut source: impl FnMut() -> StatisticsSample,
    mut take_interval: impl FnMut() -> IntervalTaken,
    mut sink: impl FnMut(&str),
) -> ReporterOutcome {
    let name = settings.name;
    let period_nanos = settings.period_millis.max(1).saturating_mul(NANOS_PER_MILLI);
    let mut previous = source();
    let mut previous_time = (clock.now_nanos)();
    let mut collector = Collector {
        settings,
        outcome: ReporterOutcome::default(),
        interval_start: StatisticsSample::default(),
        publish_nanos_total: 0,
    };
    collector.outcome.first_runtime = previous.runtime.map(|sample| TimedRuntime {
        sample,
        epoch_nanos: previous_time,
    });
    let mut primed = false;
    let mut print = |previous: &StatisticsSample, current: &StatisticsSample, length: i64| {
        let mut text = format_statistics(
            settings.side,
            name,
            &deltas(previous, current),
            length,
            current.connections,
        );
        if let (Some(before), Some(now)) = (&previous.runtime, &current.runtime) {
            text.push('\n');
            text.push_str(&format_runtime(name, before, now, length));
        }
        text.push('\n');
        sink(&text);
    };
    // The next tick is always after the current time, so a late wake-up never moves the later ticks.
    while (clock.sleep_until_nanos)(((clock.now_nanos)() / period_nanos + 1) * period_nanos) {
        let current = source();
        let current_time = (clock.now_nanos)();
        if primed {
            print(&previous, &current, current_time - previous_time);
        }
        collector.push(&current, previous_time, current_time, take_interval());
        primed = true;
        previous = current;
        previous_time = current_time;
    }
    let current = source();
    let current_time = (clock.now_nanos)();
    // A shorter last block is mostly the noise of its few batches.
    if current_time - previous_time >= MIN_LAST_BLOCK_NANOS {
        print(&previous, &current, current_time - previous_time);
    }
    collector.push(&current, previous_time, current_time, take_interval());
    collector.outcome.last_runtime = current.runtime.map(|sample| TimedRuntime {
        sample,
        epoch_nanos: current_time,
    });
    collector.outcome
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use regex::Regex;
    use serde_json::Value;

    use super::super::kit::kit_json;
    use super::*;

    const SECOND: i64 = NANOS_PER_SECOND;

    fn lines(text: &str) -> Vec<&str> {
        text.lines().collect()
    }

    fn totals(pairs: &[(i64, i64)]) -> Vec<InstanceTotals> {
        pairs
            .iter()
            .map(|&(records, weighted_nanos)| InstanceTotals {
                records,
                weighted_nanos,
            })
            .collect()
    }

    fn producer(name: &str, deltas: &[InstanceTotals], period_nanos: i64, connections: i64) -> String {
        format_statistics(Side::Producer, name, deltas, period_nanos, connections)
    }

    /// A sample with a heap of 120 MB of 256 MB.
    fn runtime(young_count: i64, young_ms: i64, full_count: i64, full_ms: i64, cpu_nanos: i64) -> RuntimeSample {
        let value = |value: i64| (value >= 0).then_some(value);
        RuntimeSample {
            heap_used_bytes: Some(120 * BYTES_PER_MB),
            heap_committed_bytes: Some(256 * BYTES_PER_MB),
            young_gc_count: value(young_count),
            young_gc_time_ms: value(young_ms),
            full_gc_count: value(full_count),
            full_gc_time_ms: value(full_ms),
            process_cpu_time_ns: value(cpu_nanos),
        }
    }

    // The kit.

    fn int(value: &Value) -> i64 {
        value
            .as_i64()
            .unwrap_or_else(|| panic!("an integer was expected: {value}"))
    }

    fn optional(value: &Value) -> Option<i64> {
        (!value.is_null()).then(|| int(value))
    }

    fn kit_instances(entry: &Value) -> Vec<InstanceTotals> {
        let key = if entry["side"] == "producer" {
            "weighted_ns"
        } else {
            "lag_sum_ns"
        };
        entry["instances"]
            .as_array()
            .expect("the instances")
            .iter()
            .map(|instance| InstanceTotals {
                records: int(&instance["records"]),
                weighted_nanos: int(&instance[key]),
            })
            .collect()
    }

    /// The kit sample of the runtime: its `gc_count` is the sum of the young and the full collections, so the young
    /// ones carry it and the full ones stay at 0 while it is present.
    fn kit_runtime(sample: &Value) -> RuntimeSample {
        let gc_count = optional(&sample["gc_count"]);
        let gc_time = optional(&sample["gc_time_ms"]);
        RuntimeSample {
            heap_used_bytes: optional(&sample["heap_used_bytes"]),
            heap_committed_bytes: optional(&sample["heap_committed_bytes"]),
            young_gc_count: gc_count,
            young_gc_time_ms: gc_time,
            full_gc_count: gc_count.map(|_| 0),
            full_gc_time_ms: gc_time.map(|_| 0),
            process_cpu_time_ns: optional(&sample["cpu_ns"]),
        }
    }

    fn kit_regex(name: &str) -> Regex {
        let kit = kit_json("regexes.json");
        let expression = kit["regexes"][name]
            .as_str()
            .unwrap_or_else(|| panic!("no expression {name} in the kit"));
        Regex::new(expression).unwrap_or_else(|error| panic!("the expression {name} does not compile: {error}"))
    }

    #[test]
    fn the_grouping_of_the_kit() {
        let kit = kit_json("lines.json");
        let samples = kit["grouping"].as_array().expect("the grouping");
        assert!(samples.len() >= 12);
        for sample in samples {
            assert_eq!(
                group_digits(int(&sample["value"])),
                sample["text"].as_str().expect("the text"),
                "{sample}"
            );
        }
    }

    #[test]
    fn the_statistics_blocks_of_the_kit_byte_for_byte() {
        let kit = kit_json("lines.json");
        let blocks = kit["statistics"].as_array().expect("the statistics");
        assert!(blocks.len() >= 6);
        let mut sides = [0, 0];
        for block in blocks {
            let side = if block["side"] == "producer" {
                sides[0] += 1;
                Side::Producer
            } else {
                sides[1] += 1;
                Side::Consumer
            };
            let text = format_statistics(
                side,
                block["name"].as_str().expect("the name"),
                &kit_instances(block),
                int(&block["interval_ns"]),
                int(&block["connections"]),
            );
            assert_eq!(text, block["text"].as_str().expect("the text"), "{block}");
        }
        assert!(sides[0] >= 4 && sides[1] >= 2, "{sides:?}");
    }

    #[test]
    fn the_runtime_lines_of_the_kit_byte_for_byte() {
        let kit = kit_json("lines.json");
        let samples = kit["runtime"].as_array().expect("the runtime lines");
        assert!(samples.len() >= 4);
        for sample in samples {
            let line = format_runtime(
                sample["name"].as_str().expect("the name"),
                &kit_runtime(&sample["previous"]),
                &kit_runtime(&sample["current"]),
                int(&sample["interval_ns"]),
            );
            assert_eq!(line, sample["text"].as_str().expect("the text"), "{sample}");
        }
    }

    #[test]
    fn the_expressions_of_the_kit_match_every_sample_and_the_captured_lines() {
        let (summary, instance, runtime_line) = (kit_regex("summary"), kit_regex("instance"), kit_regex("runtime"));
        let kit = kit_json("lines.json");
        for block in kit["statistics"].as_array().expect("the statistics") {
            let text = block["text"].as_str().expect("the text");
            let block_lines = lines(text);
            let captures = summary
                .captures(block_lines[0])
                .unwrap_or_else(|| panic!("the summary expression does not match {}", block_lines[0]));
            let side = if block["side"] == "producer" {
                "Producer"
            } else {
                "Consumer"
            };
            assert_eq!(&captures[1], side);
            assert_eq!(&captures[2], block["name"].as_str().expect("the name"));
            assert_eq!(
                &captures[5],
                block["instances"].as_array().expect("the instances").len().to_string()
            );
            assert_eq!(&captures[6], block["connections"].to_string());
            assert!(!instance.is_match(block_lines[0]));
            for (index, line) in block_lines[1..].iter().enumerate() {
                let captures = instance
                    .captures(line)
                    .unwrap_or_else(|| panic!("the instance expression does not match {line}"));
                assert_eq!(&captures[1], block["name"].as_str().expect("the name"));
                assert_eq!(&captures[2], (index + 1).to_string());
                assert!(!summary.is_match(line));
            }
            assert_eq!(
                block_lines.len(),
                1 + block["instances"].as_array().expect("the instances").len()
            );
        }
        for sample in kit["runtime"].as_array().expect("the runtime lines") {
            let line = sample["text"].as_str().expect("the text");
            let captures = runtime_line
                .captures(line)
                .unwrap_or_else(|| panic!("the runtime expression does not match {line}"));
            assert_eq!(&captures[1], sample["name"].as_str().expect("the name"));
            assert!(!summary.is_match(line) && !instance.is_match(line));
        }
        let rejection = kit_regex("isolate_rejection");
        let mut kinds = [0; 4];
        for line in kit["captured"].as_array().expect("the captured lines") {
            let line = line.as_str().expect("a line");
            let matched = [
                summary.is_match(line),
                instance.is_match(line),
                runtime_line.is_match(line),
                rejection.is_match(line),
            ];
            assert_eq!(matched.iter().filter(|&&matched| matched).count(), 1, "{line}");
            for (kind, matched) in kinds.iter_mut().zip(matched) {
                *kind += i32::from(matched);
            }
        }
        assert!(kinds.iter().all(|&count| count > 0), "{kinds:?}");
    }

    #[test]
    fn what_the_formatter_prints_matches_the_expressions_of_the_kit() {
        let (summary, instance, runtime_line) = (kit_regex("summary"), kit_regex("instance"), kit_regex("runtime"));
        for side in [Side::Producer, Side::Consumer] {
            let text = format_statistics(side, "a b", &totals(&[(1_234_567, -41_000), (0, 0)]), SECOND, 3);
            let text = lines(&text);
            assert!(summary.is_match(text[0]), "{}", text[0]);
            assert!(instance.is_match(text[1]) && instance.is_match(text[2]), "{text:?}");
        }
        let default = RuntimeSample::default();
        let line = format_runtime("a b", &default, &default, SECOND);
        assert!(runtime_line.is_match(&line), "{line}");
    }

    // The formatter.

    #[test]
    fn digit_grouping() {
        assert_eq!(group_digits(1_000_000_000_000), "1,000,000,000,000");
        assert_eq!(group_digits(-1234), "-1,234");
        assert_eq!(group_digits(i64::MIN), "-9,223,372,036,854,775,808");
        assert_eq!(group_digits(i64::MAX), "9,223,372,036,854,775,807");
    }

    #[test]
    fn the_block_of_1_234_567_records_over_10_seconds() {
        let text = producer(
            "NetTest",
            &totals(&[(600_000, 600_000 * 20_000), (634_567, 634_567 * 20_000)]),
            10 * SECOND,
            1,
        );
        assert_eq!(
            lines(&text),
            [
                "{*NetTestProducerSide-NetTest*} data 123,456 rps, lag 20 us, instances 2, connections 1",
                "    NetTest.1 data 60,000 rps, lag 20 us",
                "    NetTest.2 data 63,456 rps, lag 20 us",
            ]
        );
        assert!(!text.ends_with('\n'));
    }

    #[test]
    fn zero_records_give_zero_rate_and_lag_and_the_name_is_in_both_labels() {
        for side in [Side::Producer, Side::Consumer] {
            let text = format_statistics(side, "NetTest", &totals(&[(0, 0)]), SECOND, 0);
            assert_eq!(lines(&text).len(), 2, "one instance has its own line");
            assert!(lines(&text)[0].contains(" data 0 rps, lag 0 us, instances 1, connections 0"));
            assert_eq!(lines(&text)[1], "    NetTest.1 data 0 rps, lag 0 us");
        }
        let named = producer("Bench", &totals(&[(10, 0), (10, 0)]), SECOND, 0);
        let named = lines(&named);
        assert!(named[0].starts_with("{*NetTestProducerSide-Bench*} data 20 rps"));
        assert!(named[1].starts_with("    Bench.1 data 10 rps"));
        assert!(named[2].starts_with("    Bench.2 data 10 rps"));
    }

    #[test]
    fn the_lag_is_record_weighted_on_the_producer() {
        assert_eq!(lag_micros(200, 100 * 10_000 + 100 * 30_000), 20);
        assert_eq!(lag_micros(400, 300 * 10_000 + 100 * 50_000), 20);
        assert_eq!(lag_micros(0, 0), 0);
        let text = producer("NetTest", &totals(&[(400, 300 * 10_000 + 100 * 50_000)]), SECOND, 0);
        assert!(lines(&text)[0].contains("lag 20 us"));
    }

    #[test]
    fn the_consumer_lag_is_the_mean_divided_toward_zero_and_may_be_negative() {
        assert_eq!(mean_lag_micros(1000, -12_345_678), -12);
        assert_eq!(mean_lag_micros(3, -2999), 0);
        assert_eq!(mean_lag_micros(1, -1999), -1);
        assert_eq!(mean_lag_micros(1, 1999), 1);
        assert_eq!(mean_lag_micros(0, -5), 0);
        // A producer lag below zero is 0, a consumer lag keeps its sign.
        let negative = totals(&[(10, -50_000)]);
        assert!(lines(&producer("NetTest", &negative, SECOND, 0))[0].contains("lag 0 us"));
        let text = format_statistics(Side::Consumer, "NetTest", &negative, SECOND, 0);
        assert!(lines(&text)[0].starts_with("{*NetTestConsumerSide-NetTest*} data 10 rps, lag -5 us"));
        assert_eq!(lines(&text)[1], "    NetTest.1 data 10 rps, lag -5 us");
    }

    #[test]
    fn the_rate_uses_the_real_period_length() {
        assert_eq!(records_per_second(1000, SECOND), 1000);
        assert_eq!(records_per_second(500, SECOND / 2), 1000);
        assert_eq!(records_per_second(1, 3 * SECOND), 0);
        assert_eq!(records_per_second(1000, 0), 0);
        assert_eq!(records_per_second(9_000_000_000_000, 1000 * SECOND), 9_000_000_000);
    }

    #[test]
    fn the_runtime_line_sums_young_and_full_collections() {
        assert_eq!(
            format_runtime(
                "NetTest",
                &runtime(2, 10, 0, 0, 0),
                &runtime(5, 25, 0, 0, SECOND),
                SECOND
            ),
            "{*NetTestRuntime-NetTest*} heap 120 MB of 256 MB, gc 3 (15 ms), cpu 100%"
        );
        assert_eq!(
            format_runtime("Bench", &runtime(2, 10, 1, 40, 0), &runtime(5, 25, 2, 100, 0), SECOND),
            "{*NetTestRuntime-Bench*} heap 120 MB of 256 MB, gc 4 (75 ms), cpu 0%"
        );
    }

    #[test]
    fn absent_or_decreasing_gc_values_print_n_a() {
        let line = format_runtime(
            "NetTest",
            &runtime(-1, -1, -1, -1, 0),
            &runtime(-1, -1, -1, -1, 0),
            SECOND,
        );
        assert_eq!(line, "{*NetTestRuntime-NetTest*} heap 120 MB of 256 MB, gc n/a, cpu 0%");
        let gc = |previous: RuntimeSample, current: RuntimeSample| {
            format_runtime("NetTest", &previous, &current, SECOND).contains("gc n/a")
        };
        assert!(gc(runtime(-1, -1, 0, 0, 0), runtime(5, 25, 0, 0, 0)));
        assert!(gc(runtime(2, 10, -1, -1, 0), runtime(5, 25, -1, -1, 0)));
        assert!(gc(runtime(5, 25, 0, 0, 0), runtime(2, 10, 0, 0, 0)));
    }

    #[test]
    fn the_cpu_load_is_the_cpu_time_of_the_period_over_its_length() {
        let cpu = |previous: i64, current: i64, period: i64| {
            let line = format_runtime(
                "NetTest",
                &runtime(0, 0, 0, 0, previous),
                &runtime(0, 0, 0, 0, current),
                period,
            );
            line[line.rfind(", cpu ").unwrap() + 6..].to_owned()
        };
        assert_eq!(cpu(0, SECOND * 3 / 2, SECOND), "150%");
        assert_eq!(cpu(SECOND, SECOND + SECOND / 4, SECOND), "25%");
        assert_eq!(cpu(0, 25 * SECOND, 10 * SECOND), "250%");
        assert_eq!(cpu(-1, SECOND, SECOND), "n/a");
        assert_eq!(cpu(0, -1, SECOND), "n/a");
        assert_eq!(cpu(0, SECOND, 0), "n/a");
    }

    #[test]
    fn an_absent_heap_prints_n_a() {
        let mut current = runtime(0, 0, 0, 0, 0);
        current.heap_used_bytes = None;
        assert_eq!(
            format_runtime("NetTest", &runtime(0, 0, 0, 0, 0), &current, SECOND),
            "{*NetTestRuntime-NetTest*} heap n/a of 256 MB, gc 0 (0 ms), cpu 0%"
        );
        current.heap_committed_bytes = None;
        assert_eq!(
            format_runtime("NetTest", &runtime(0, 0, 0, 0, 0), &current, SECOND),
            "{*NetTestRuntime-NetTest*} heap n/a of n/a, gc 0 (0 ms), cpu 0%"
        );
        assert_eq!(
            format_runtime("NetTest", &RuntimeSample::default(), &RuntimeSample::default(), SECOND),
            "{*NetTestRuntime-NetTest*} heap n/a of n/a, gc n/a, cpu n/a"
        );
    }

    // The lags.

    #[test]
    fn the_recorder_keeps_sums_signs_the_largest_and_the_histogram_and_starts_over_at_take() {
        let recorder = LagRecorder::default();
        recorder.record([1_000, -500, 2_000]);
        recorder.record([i64::MAX / 4]);
        let first = recorder.take();
        assert_eq!(
            first.stats,
            LagStats {
                count: 4,
                negative: 1,
                sum: 2_500 + i64::MAX / 4,
                max: i64::MAX / 4,
            }
        );
        assert_eq!(first.histogram.count(), 4);
        // The negative lag is 0 in the histogram, so the smallest percentile is 0.
        assert_eq!(first.histogram.value_at_percentile(25.0), 0);
        let second = recorder.take();
        assert_eq!(second.stats, LagStats::default());
        assert_eq!(second.histogram.count(), 0);
        // A first lag below zero is the largest of an interval of negative lags only.
        recorder.record([-7, -3]);
        let negative = recorder.take();
        assert_eq!((negative.stats.max, negative.stats.negative), (-3, 2));
    }

    #[test]
    fn the_recorder_takes_lags_of_several_threads_without_loss() {
        let recorder = LagRecorder::default();
        std::thread::scope(|scope| {
            for thread in 0..4 {
                let recorder = &recorder;
                scope.spawn(move || {
                    for batch in 0..100 {
                        recorder.record((0..10).map(|index| thread * 1000 + batch + index));
                    }
                });
            }
        });
        let lags = recorder.take();
        assert_eq!(lags.stats.count, 4000);
        assert_eq!(lags.histogram.count(), 4000);
        assert_eq!(lags.stats.negative, 0);
    }

    #[test]
    fn lag_statistics_add_up() {
        let mut total = LagStats::default();
        total.add(&LagStats::default());
        assert_eq!(total, LagStats::default());
        total.add(&LagStats {
            count: 2,
            negative: 1,
            sum: -5,
            max: -1,
        });
        assert_eq!(total.max, -1, "the largest of negative lags only is negative");
        total.add(&LagStats {
            count: 1,
            negative: 0,
            sum: 10,
            max: 10,
        });
        assert_eq!(
            total,
            LagStats {
                count: 3,
                negative: 1,
                sum: 5,
                max: 10
            }
        );
    }

    // The warm-up.

    #[test]
    fn the_warm_up_ends_at_the_start_plus_the_warm_up_and_an_overlapping_interval_is_warm_up() {
        let warmup = 30 * SECOND;
        let start = 1000 * SECOND;
        // Before the start of the warm-up (the first record has not come) everything is warm-up.
        assert!(is_warmup_interval(warmup, None, start));
        assert!(is_warmup_interval(warmup, Some(0), start));
        assert!(is_warmup_interval(warmup, Some(start), start - 1));
        assert!(is_warmup_interval(warmup, Some(start), start));
        // An interval that starts before the end of the warm-up overlaps it or lies in it.
        assert!(is_warmup_interval(warmup, Some(start), start + warmup - 1));
        assert!(!is_warmup_interval(warmup, Some(start), start + warmup));
        assert!(!is_warmup_interval(warmup, Some(start), start + warmup + 1));
    }

    // The reporter.

    /// A clock that jumps: each sleep sets the time to its target and ends the reporter when the target is after the
    /// stop time (the time then becomes the stop time).
    struct FakeClock {
        now: Cell<i64>,
        stop_at: i64,
        targets: RefCell<Vec<i64>>,
    }

    impl FakeClock {
        fn new(now: i64, stop_at: i64) -> Self {
            FakeClock {
                now: Cell::new(now),
                stop_at,
                targets: RefCell::new(Vec::new()),
            }
        }

        fn sleep(&self, target: i64) -> bool {
            self.targets.borrow_mut().push(target);
            if target > self.stop_at {
                self.now.set(self.stop_at);
                return false;
            }
            self.now.set(target);
            true
        }

        fn clock(&self) -> ReporterClock<impl Fn() -> i64 + '_, impl FnMut(i64) -> bool + '_> {
            ReporterClock {
                now_nanos: || self.now.get(),
                sleep_until_nanos: |target| self.sleep(target),
            }
        }
    }

    fn settings(period_millis: i64) -> ReporterSettings<'static> {
        ReporterSettings {
            name: "NetTest",
            side: Side::Producer,
            period_millis,
            warmup_millis: None,
        }
    }

    fn nothing() -> IntervalTaken {
        IntervalTaken::default()
    }

    #[test]
    fn the_reporter_starts_the_count_on_the_first_tick_and_prints_on_the_whole_seconds_after_it() {
        let start = 1000 * SECOND + SECOND / 3;
        let fake = FakeClock::new(start, start + 3 * SECOND + SECOND / 6);
        let mut printed = Vec::new();
        let source = || {
            // One record per millisecond, 20 us each.
            let records = (fake.now.get() - start) / 1_000_000;
            StatisticsSample {
                instances: totals(&[(records, records * 20_000)]),
                connections: 2,
                runtime: None,
            }
        };
        let outcome = run_reporter(settings(1000), fake.clock(), source, nothing, |text| {
            printed.push((fake.now.get(), text.to_owned()))
        });
        assert_eq!(
            *fake.targets.borrow(),
            [1001 * SECOND, 1002 * SECOND, 1003 * SECOND, 1004 * SECOND]
        );
        assert_eq!(
            printed.iter().map(|(time, _)| *time).collect::<Vec<_>>(),
            [1002 * SECOND, 1003 * SECOND]
        );
        for (_, text) in &printed {
            assert!(text.ends_with('\n'));
            assert_eq!(
                lines(text),
                [
                    "{*NetTestProducerSide-NetTest*} data 1,000 rps, lag 20 us, instances 1, connections 2",
                    "    NetTest.1 data 1,000 rps, lag 20 us",
                ]
            );
        }
        // The first tick printed nothing but closed the first interval; the last 1/6 s was recorded, not printed.
        let bounds: Vec<_> = outcome
            .intervals
            .iter()
            .map(|interval| (interval.start_epoch_nanos, interval.end_epoch_nanos))
            .collect();
        assert_eq!(
            bounds,
            [
                (start, 1001 * SECOND),
                (1001 * SECOND, 1002 * SECOND),
                (1002 * SECOND, 1003 * SECOND),
                (1003 * SECOND, start + 3 * SECOND + SECOND / 6),
            ]
        );
    }

    #[test]
    fn with_runtime_values_the_runtime_line_follows_each_block() {
        let start = 1000 * SECOND;
        // Ticks of 2 s at 1002 (starts the count), 1004, then the last block of 1.5 s.
        let fake = FakeClock::new(start, start + 5 * SECOND + SECOND / 2);
        let mut printed = Vec::new();
        let source = || {
            // 2 young collections of 5 ms each in every whole second, and half a core of CPU time.
            let elapsed = fake.now.get() - start;
            let seconds = elapsed / SECOND;
            StatisticsSample {
                instances: totals(&[(0, 0)]),
                connections: 0,
                runtime: Some(runtime(2 * seconds, 10 * seconds, 0, 0, elapsed / 2)),
            }
        };
        let outcome = run_reporter(settings(2000), fake.clock(), source, nothing, |text| {
            printed.push(text.to_owned())
        });
        assert_eq!(printed.len(), 2);
        let summary = kit_regex("summary");
        let runtime_line = kit_regex("runtime");
        for text in &printed {
            let block = lines(text);
            assert_eq!(block.len(), 3);
            assert!(summary.is_match(block[0]));
            assert!(runtime_line.is_match(block[2]));
        }
        assert_eq!(
            lines(&printed[0])[2],
            "{*NetTestRuntime-NetTest*} heap 120 MB of 256 MB, gc 4 (20 ms), cpu 50%"
        );
        // The last block of 1.5 s holds one whole second of collections; the CPU load uses its real length.
        assert_eq!(
            lines(&printed[1])[2],
            "{*NetTestRuntime-NetTest*} heap 120 MB of 256 MB, gc 2 (10 ms), cpu 50%"
        );
        // The first and the last runtime samples are kept with their times.
        let first = outcome.first_runtime.expect("the first sample");
        let last = outcome.last_runtime.expect("the last sample");
        assert_eq!(first.epoch_nanos, start);
        assert_eq!(first.sample, runtime(0, 0, 0, 0, 0));
        assert_eq!(last.epoch_nanos, start + 5 * SECOND + SECOND / 2);
        assert_eq!(last.sample.process_cpu_time_ns, Some(5 * SECOND / 2 + SECOND / 4));
    }

    /// Runs the reporter from the start until the stop; 300 records of 5 us arrive at the first sleep.
    fn run_once(period_millis: i64, start: i64, stop_at: i64) -> (Vec<(i64, String)>, ReporterOutcome) {
        let fake = FakeClock::new(start, stop_at);
        let records = Cell::new(0);
        let mut printed = Vec::new();
        let clock = ReporterClock {
            now_nanos: || fake.now.get(),
            sleep_until_nanos: |target| {
                records.set(300);
                fake.sleep(target)
            },
        };
        let outcome = run_reporter(
            settings(period_millis),
            clock,
            || StatisticsSample {
                instances: totals(&[(records.get(), records.get() * 5000)]),
                connections: 0,
                runtime: None,
            },
            nothing,
            |text| printed.push((fake.now.get(), text.to_owned())),
        );
        (printed, outcome)
    }

    #[test]
    fn a_stop_before_the_first_tick_prints_the_whole_run() {
        let (printed, outcome) = run_once(10_000, 2000 * SECOND, 2000 * SECOND + SECOND * 3 / 2);
        assert_eq!(printed.len(), 1);
        assert_eq!(printed[0].0, 2000 * SECOND + SECOND * 3 / 2);
        assert_eq!(
            lines(&printed[0].1)[0],
            "{*NetTestProducerSide-NetTest*} data 200 rps, lag 5 us, instances 1, connections 0"
        );
        assert_eq!(outcome.intervals.len(), 1);
        assert_eq!(outcome.intervals[0].records, 300);
        assert_eq!(outcome.intervals[0].rate, 200);
    }

    #[test]
    fn a_last_block_shorter_than_one_second_is_not_printed_but_recorded() {
        let (printed, outcome) = run_once(1000, 2000 * SECOND, 2000 * SECOND + SECOND / 2);
        assert!(printed.is_empty());
        assert_eq!(
            outcome.intervals.len(),
            1,
            "the run is recorded though nothing is printed"
        );
        assert_eq!(
            outcome.intervals[0].end_epoch_nanos - outcome.intervals[0].start_epoch_nanos,
            SECOND / 2
        );
        assert_eq!(outcome.intervals[0].records, 300);
        // A tick at 2005 starts the count, one at 2010 prints; 999 ms after it the stop prints nothing, 1 s after it a
        // block; the intervals are three in both cases (to the first tick, to the second, to the stop).
        let (printed, outcome) = run_once(5000, 2000 * SECOND, 2011 * SECOND - 1_000_000);
        assert_eq!((printed.len(), outcome.intervals.len()), (1, 3));
        let (printed, outcome) = run_once(5000, 2000 * SECOND, 2011 * SECOND);
        assert_eq!((printed.len(), outcome.intervals.len()), (2, 3));
    }

    #[test]
    fn the_intervals_add_up_to_the_records_and_cover_the_run() {
        let start = 500 * SECOND + SECOND / 7;
        let stop = start + 7 * SECOND + SECOND / 5;
        let fake = FakeClock::new(start, stop);
        let source = || {
            // Two instances, 1000 and 250 records per second, steady.
            let elapsed = fake.now.get() - start;
            StatisticsSample {
                instances: totals(&[(elapsed / 1_000_000, 0), (elapsed / 4_000_000, 0)]),
                connections: 1,
                runtime: None,
            }
        };
        let outcome = run_reporter(settings(2000), fake.clock(), source, nothing, |_| {});
        let final_sample = source();
        let records: i64 = outcome.intervals.iter().map(|interval| interval.records).sum();
        assert_eq!(
            records,
            final_sample
                .instances
                .iter()
                .map(|instance| instance.records)
                .sum::<i64>()
        );
        assert!(records > 0);
        assert_eq!(outcome.intervals.first().expect("an interval").start_epoch_nanos, start);
        assert_eq!(outcome.intervals.last().expect("an interval").end_epoch_nanos, stop);
        for pair in outcome.intervals.windows(2) {
            assert_eq!(pair[0].end_epoch_nanos, pair[1].start_epoch_nanos);
        }
        // The ticks are at 502, 504 and 506; the stop at 507.34 closes the fourth interval.
        assert_eq!(outcome.intervals.len(), 4);
        assert!(outcome.intervals.iter().all(|interval| interval.connections == 1));
        assert_eq!(outcome.intervals[0].start_epoch_ms(), start / 1_000_000);
        assert_eq!(outcome.intervals[3].end_epoch_ms(), stop / 1_000_000);
        assert_eq!(outcome.intervals[1].records, 2 * 1000 + 2 * 250);
        assert_eq!(outcome.intervals[1].rate, 1250);
    }

    #[test]
    fn the_phase_of_the_intervals_follows_the_warm_up_that_starts_at_the_first_grant() {
        // Ticks every second from 101; the warm-up of 2 s starts at 101.5 (the first batch, which the reporter learns
        // at the tick of 102), so it ends at 103.5.
        let fake = FakeClock::new(100 * SECOND + SECOND / 2, 106 * SECOND + SECOND / 2);
        let first_batch = Cell::new(None);
        let source = || {
            if fake.now.get() >= 102 * SECOND && first_batch.get().is_none() {
                first_batch.set(Some(101 * SECOND + SECOND / 2));
            }
            StatisticsSample {
                instances: totals(&[(0, 0)]),
                ..StatisticsSample::default()
            }
        };
        let take = || IntervalTaken {
            warmup_start_epoch_nanos: first_batch.get(),
            ..IntervalTaken::default()
        };
        let outcome = run_reporter(
            ReporterSettings {
                warmup_millis: Some(2000),
                ..settings(1000)
            },
            fake.clock(),
            source,
            take,
            |_| {},
        );
        let phases: Vec<_> = outcome
            .intervals
            .iter()
            .map(|interval| (interval.start_epoch_nanos / SECOND, interval.warmup))
            .collect();
        // The interval from 100.5 ends before the warm-up is known to have started, so it is warm-up as well.
        assert_eq!(
            phases,
            [
                (100, true),
                (101, true),
                (102, true),
                // 103 starts before 103.5, the end of the warm-up: it overlaps the end, so it is warm-up.
                (103, true),
                (104, false),
                (105, false),
                (106, false),
            ]
        );
    }

    #[test]
    fn without_a_warm_up_every_interval_is_measured() {
        let fake = FakeClock::new(100 * SECOND, 103 * SECOND);
        let outcome = run_reporter(settings(1000), fake.clock(), StatisticsSample::default, nothing, |_| {});
        // Ticks at 101, 102 and 103, then the stop closes an interval of 0 s.
        assert_eq!(outcome.intervals.len(), 4);
        assert!(outcome.intervals.iter().all(|interval| !interval.warmup));
    }

    fn consumer_settings(warmup_millis: Option<i64>) -> ReporterSettings<'static> {
        ReporterSettings {
            side: Side::Consumer,
            warmup_millis,
            ..settings(1000)
        }
    }

    #[test]
    fn the_consumer_reporter_prints_the_consumer_block_and_leaves_the_warm_up_out_of_the_totals() {
        // Ticks at 101 to 105; the warm-up of 2 s starts at 100, so the intervals from 100 and 101 are warm-up and
        // the one from 102 is measured.
        let fake = FakeClock::new(100 * SECOND, 105 * SECOND + SECOND / 2);
        let recorder = LagRecorder::default();
        let mut printed = Vec::new();
        let source = || {
            // Each interval holds 100 events: the sum of the lags grows by 100 * the lag of the interval.
            let intervals = (fake.now.get() / SECOND) - 100;
            StatisticsSample {
                instances: totals(&[(100 * intervals, intervals * 100 * 1_000_000)]),
                connections: 1,
                runtime: None,
            }
        };
        let clock = ReporterClock {
            now_nanos: || fake.now.get(),
            sleep_until_nanos: |target| {
                // The listeners record 100 lags of 1 ms * (the number of the interval) just before the tick.
                let number = target / SECOND - 100;
                if number <= 5 {
                    recorder.record((0..100).map(|index| number * 1_000_000 + index));
                }
                fake.sleep(target)
            },
        };
        let outcome = run_reporter(
            consumer_settings(Some(2000)),
            clock,
            source,
            || IntervalTaken {
                warmup_start_epoch_nanos: Some(100 * SECOND),
                lags: Some(recorder.take()),
                ..IntervalTaken::default()
            },
            |text| printed.push(text.to_owned()),
        );
        assert_eq!(printed.len(), 4);
        assert_eq!(
            lines(&printed[0]),
            [
                "{*NetTestConsumerSide-NetTest*} data 100 rps, lag 1,000 us, instances 1, connections 1",
                "    NetTest.1 data 100 rps, lag 1,000 us",
            ]
        );
        let phases: Vec<_> = outcome.intervals.iter().map(|interval| interval.warmup).collect();
        assert_eq!(phases, [true, true, false, false, false, false]);
        // 5 intervals of 100 events and a last one of 0.5 s without events: the measured ones hold the lags of the
        // intervals 3 to 5.
        assert_eq!(outcome.e2e.stats.count, 300);
        assert_eq!(outcome.e2e.histogram.count(), 300);
        assert_eq!(outcome.e2e.stats.negative, 0);
        assert_eq!(outcome.e2e.stats.max, 5 * 1_000_000 + 99);
        assert_eq!(outcome.e2e.stats.sum, (3 + 4 + 5) * 100 * 1_000_000 + 3 * 4950);
        let record = &outcome.intervals[2];
        assert_eq!(record.records, 100);
        assert_eq!(record.lag_max_ns, Some(3 * 1_000_000 + 99));
        assert_eq!(record.lag_mean_ns, Some(3 * 1_000_000 + 49));
        // The percentile of the interval is the highest equivalent value of its bucket: within 0.1% above.
        let p50 = record.lag_p50_ns.expect("the median");
        assert!((3_000_049..=3_000_049 + 3_000).contains(&p50), "{p50}");
        let p99 = record.lag_p99_ns.expect("the 99th percentile");
        assert!((3_000_098..=3_000_098 + 3_000).contains(&p99), "{p99}");
        assert_eq!(record.publish_ns_per_event, None);
        assert_eq!((record.slip_max_ns, record.slip_end_ns), (None, None));
        // The warm-up intervals keep their own lag fields.
        assert_eq!(outcome.intervals[0].lag_max_ns, Some(99 + 1_000_000));
    }

    #[test]
    fn an_interval_without_lags_has_none_and_the_totals_stay_empty() {
        let fake = FakeClock::new(100 * SECOND, 102 * SECOND);
        let recorder = LagRecorder::default();
        let outcome = run_reporter(
            consumer_settings(None),
            fake.clock(),
            StatisticsSample::default,
            || IntervalTaken {
                lags: Some(recorder.take()),
                ..IntervalTaken::default()
            },
            |_| {},
        );
        assert_eq!(outcome.intervals.len(), 3);
        for interval in &outcome.intervals {
            assert_eq!(
                (
                    interval.lag_mean_ns,
                    interval.lag_p50_ns,
                    interval.lag_p99_ns,
                    interval.lag_max_ns
                ),
                (None, None, None, None)
            );
        }
        assert_eq!(outcome.e2e.stats, LagStats::default());
        assert_eq!(outcome.e2e.histogram.count(), 0);
    }

    #[test]
    fn negative_lags_keep_their_sign_in_the_mean_and_the_maximum_and_are_zero_in_the_histogram() {
        let fake = FakeClock::new(100 * SECOND, 101 * SECOND);
        let recorder = LagRecorder::default();
        let recorded = Cell::new(false);
        let clock = ReporterClock {
            now_nanos: || fake.now.get(),
            sleep_until_nanos: |target| {
                if !recorded.replace(true) {
                    recorder.record([-4_000, -2_000, 0]);
                }
                fake.sleep(target)
            },
        };
        let outcome = run_reporter(
            consumer_settings(None),
            clock,
            StatisticsSample::default,
            || IntervalTaken {
                lags: Some(recorder.take()),
                ..IntervalTaken::default()
            },
            |_| {},
        );
        let interval = &outcome.intervals[0];
        assert_eq!(interval.lag_mean_ns, Some(-2_000));
        assert_eq!(interval.lag_max_ns, Some(0));
        assert_eq!(interval.lag_p50_ns, Some(0));
        assert_eq!(outcome.e2e.stats.negative, 2);
        assert_eq!(outcome.e2e.stats.sum, -6_000);
    }

    #[test]
    fn the_producer_intervals_carry_the_publish_time_per_event_and_the_slip() {
        let fake = FakeClock::new(100 * SECOND, 103 * SECOND);
        let clock_now = || fake.now.get();
        let step = Cell::new(0_i64);
        let outcome = run_reporter(
            settings(1000),
            ReporterClock {
                now_nanos: clock_now,
                sleep_until_nanos: |target| {
                    step.set(step.get() + 1);
                    fake.sleep(target)
                },
            },
            || StatisticsSample {
                // 100 records per interval, 40 us each, except an idle interval.
                instances: totals(&[(100 * step.get().min(2), 0)]),
                connections: 1,
                runtime: None,
            },
            || IntervalTaken {
                publish_nanos_total: step.get().min(2) * 100 * 40_000,
                slip_max_nanos: Some(step.get() * 1000),
                slip_end_nanos: Some(step.get() * 10),
                ..IntervalTaken::default()
            },
            |_| {},
        );
        let published: Vec<_> = outcome
            .intervals
            .iter()
            .map(|interval| {
                (
                    interval.records,
                    interval.publish_ns_per_event,
                    interval.slip_max_ns,
                    interval.slip_end_ns,
                )
            })
            .collect();
        assert_eq!(
            published,
            [
                (100, Some(40_000), Some(1000), Some(10)),
                (100, Some(40_000), Some(2000), Some(20)),
                // No records, no time per event; the last interval (to the stop, 0 s long) is the fourth step.
                (0, None, Some(3000), Some(30)),
                (0, None, Some(4000), Some(40)),
            ]
        );
        assert_eq!(outcome.e2e.stats, LagStats::default());
    }

    #[test]
    fn a_late_wake_up_does_not_move_the_later_ticks() {
        let fake = FakeClock::new(100 * SECOND + 10, 104 * SECOND);
        let late = Cell::new(true);
        let clock = ReporterClock {
            now_nanos: || fake.now.get(),
            sleep_until_nanos: |target| {
                let result = fake.sleep(target);
                // The first wake-up is 400 ms late.
                if late.get() && result {
                    fake.now.set(target + SECOND * 4 / 10);
                    late.set(false);
                }
                result
            },
        };
        run_reporter(
            settings(1000),
            clock,
            || StatisticsSample {
                instances: totals(&[(0, 0)]),
                ..StatisticsSample::default()
            },
            nothing,
            |_| {},
        );
        assert_eq!(fake.targets.borrow()[..3], [101 * SECOND, 102 * SECOND, 103 * SECOND]);
    }

    #[test]
    fn the_system_clock_prints_aligned_blocks_until_the_stop() {
        let stop = Stop::default();
        let mut printed = Vec::new();
        let mut outcome = ReporterOutcome::default();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                std::thread::sleep(Duration::from_millis(2_600));
                stop.request();
            });
            outcome = run_reporter(
                settings(500),
                system_clock(&stop),
                || StatisticsSample {
                    instances: totals(&[(0, 0)]),
                    ..StatisticsSample::default()
                },
                nothing,
                |text| printed.push(text.to_owned()),
            );
        });
        // Ticks every 500 ms for about 2.6 s: the first starts the count, then 3 to 5 blocks.
        assert!((2..=6).contains(&printed.len()), "{printed:?}");
        // Every tick is an interval and the stop adds one.
        assert!(
            (4..=7).contains(&outcome.intervals.len()),
            "{}",
            outcome.intervals.len()
        );
    }
}
