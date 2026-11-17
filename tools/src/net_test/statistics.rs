//! The statistics of NetTest: the QD NetTest producer-side lines, the runtime line of the isolate and the reporter
//! that prints them on the ticks aligned to the wall clock.

use std::time::{Duration, SystemTime};

use dxfeed_graal::system::RuntimeStatistics;

use super::limiter::Stop;

const NANOS_PER_SECOND: i64 = 1_000_000_000;
const NANOS_PER_MICRO: i64 = 1_000;
const NANOS_PER_MILLI: i64 = 1_000_000;
const BYTES_PER_MB: i64 = 1024 * 1024;
/// The shortest last block that the reporter prints at the stop.
const MIN_LAST_BLOCK_NANOS: i64 = NANOS_PER_SECOND;
const ABSENT_TEXT: &str = "n/a";

/// The summary line of a block: `{*NetTestProducerSide-<name>*} data N rps, lag N us, instances K, connections C`.
#[cfg(test)]
pub const SUMMARY_LINE_REGEX: &str =
    "[{][*]NetTestProducerSide-[^*]+[*][}] data [0-9,]+ rps, lag [0-9,]+ us, instances [0-9]+, connections [0-9]+";
/// The runtime line after each block: `{*NetTestRuntime-<name>*} heap U MB of C MB, gc N (T ms), cpu P%`, where each
/// value may be `n/a`.
#[cfg(test)]
pub const RUNTIME_LINE_REGEX: &str = "[{][*]NetTestRuntime-[^*]+[*][}] heap ([0-9,]+ MB|n/a) of ([0-9,]+ MB|n/a), gc \
                                      ([0-9,]+ [(][0-9,]+ ms[)]|n/a), cpu ([0-9,]+%|n/a)";

/// The cumulative counters of one instance.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InstanceTotals {
    pub records: i64,
    /// The sum of records * (the time to build and publish the batch of those records) in nanoseconds.
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

/// The text of a block without the line break at the end: the summary line, then one line per instance.
pub fn format_statistics(name: &str, deltas: &[InstanceTotals], period_nanos: i64, connections: i64) -> String {
    let records: i64 = deltas.iter().map(|delta| delta.records).sum();
    let weighted_nanos: i64 = deltas.iter().map(|delta| delta.weighted_nanos).sum();
    let mut text = format!(
        "{{*NetTestProducerSide-{name}*}} data {} rps, lag {} us, instances {}, connections {connections}",
        group_digits(records_per_second(records, period_nanos)),
        group_digits(lag_micros(records, weighted_nanos)),
        deltas.len()
    );
    for (index, delta) in deltas.iter().enumerate() {
        text.push_str(&format!(
            "\n    {name}.{} data {} rps, lag {} us",
            index + 1,
            group_digits(records_per_second(delta.records, period_nanos)),
            group_digits(lag_micros(delta.records, delta.weighted_nanos))
        ));
    }
    text
}

/// The deltas of the instances between two samples.
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

/// Runs until the clock stops it. Sleeps to the next wall-clock multiple of the period and prints the deltas against
/// the previous tick; the first tick only primes the counters. At the stop it prints the partial period since the
/// last tick (or since the start) at its real length, unless it is shorter than one second. When both samples have
/// runtime values, the runtime line follows the block in the same text.
pub fn run_reporter<N: Fn() -> i64, S: FnMut(i64) -> bool>(
    name: &str,
    period_millis: i64,
    mut clock: ReporterClock<N, S>,
    mut source: impl FnMut() -> StatisticsSample,
    mut sink: impl FnMut(&str),
) {
    let period_nanos = period_millis.max(1).saturating_mul(NANOS_PER_MILLI);
    let mut previous = source();
    let mut previous_time = (clock.now_nanos)();
    let mut primed = false;
    let mut print = |previous: &StatisticsSample, current: &StatisticsSample, length: i64| {
        let mut text = format_statistics(name, &deltas(previous, current), length, current.connections);
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
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

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

    /// A whole-line match, as `std::regex_match` of the C tests.
    fn matches(expression: &str, text: &str) -> bool {
        regex::Regex::new(&format!("^(?:{expression})$"))
            .unwrap()
            .is_match(text)
    }

    #[test]
    fn digit_grouping() {
        assert_eq!(group_digits(0), "0");
        assert_eq!(group_digits(7), "7");
        assert_eq!(group_digits(999), "999");
        assert_eq!(group_digits(1000), "1,000");
        assert_eq!(group_digits(123_456), "123,456");
        assert_eq!(group_digits(1_234_567), "1,234,567");
        assert_eq!(group_digits(1_000_000_000_000), "1,000,000,000,000");
        assert_eq!(group_digits(-1234), "-1,234");
        assert_eq!(group_digits(i64::MIN), "-9,223,372,036,854,775,808");
    }

    #[test]
    fn the_block_of_1_234_567_records_over_10_seconds() {
        let text = format_statistics(
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
        assert_eq!(
            lines(&format_statistics("NetTest", &totals(&[(0, 0)]), SECOND, 0)),
            [
                "{*NetTestProducerSide-NetTest*} data 0 rps, lag 0 us, instances 1, connections 0",
                "    NetTest.1 data 0 rps, lag 0 us",
            ]
        );
        let named = format_statistics("Bench", &totals(&[(10, 0), (10, 0)]), SECOND, 0);
        let named = lines(&named);
        assert!(named[0].starts_with("{*NetTestProducerSide-Bench*} data 20 rps"));
        assert!(named[1].starts_with("    Bench.1 data 10 rps"));
        assert!(named[2].starts_with("    Bench.2 data 10 rps"));
    }

    #[test]
    fn the_lag_is_record_weighted() {
        assert_eq!(lag_micros(200, 100 * 10_000 + 100 * 30_000), 20);
        assert_eq!(lag_micros(400, 300 * 10_000 + 100 * 50_000), 20);
        assert_eq!(lag_micros(0, 0), 0);
        let text = format_statistics("NetTest", &totals(&[(400, 300 * 10_000 + 100 * 50_000)]), SECOND, 0);
        assert!(lines(&text)[0].contains("lag 20 us"));
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

    #[test]
    fn the_c_expressions_match_the_lines() {
        let block = format_statistics("NetTest", &totals(&[(1_234_567, 1_234_567 * 41_000)]), SECOND, 1);
        let summary = lines(&block)[0];
        assert!(matches(SUMMARY_LINE_REGEX, summary), "{summary}");
        assert!(!matches(SUMMARY_LINE_REGEX, lines(&block)[1]));
        for line in [
            format_runtime(
                "NetTest",
                &runtime(2, 10, 0, 0, 0),
                &runtime(5, 1025, 0, 0, SECOND),
                SECOND,
            ),
            format_runtime("NetTest", &RuntimeSample::default(), &RuntimeSample::default(), SECOND),
        ] {
            assert!(matches(RUNTIME_LINE_REGEX, &line), "{line}");
            assert!(!matches(SUMMARY_LINE_REGEX, &line), "{line}");
        }
        assert!(!matches(RUNTIME_LINE_REGEX, summary));
        assert!(!matches(
            RUNTIME_LINE_REGEX,
            "{*NetTestRuntime-NetTest*} heap 1 MB of 2 MB, gc 0 (0 ms)"
        ));
    }

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

    #[test]
    fn the_reporter_primes_on_the_first_tick_and_prints_on_the_whole_seconds_after_it() {
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
        run_reporter("NetTest", 1000, fake.clock(), source, |text| {
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
    }

    #[test]
    fn with_runtime_values_the_runtime_line_follows_each_block() {
        let start = 1000 * SECOND;
        // Ticks of 2 s at 1002 (primes), 1004, then the last block of 1.5 s.
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
        run_reporter("NetTest", 2000, fake.clock(), source, |text| {
            printed.push(text.to_owned())
        });
        assert_eq!(printed.len(), 2);
        for text in &printed {
            let block = lines(text);
            assert_eq!(block.len(), 3);
            assert!(matches(SUMMARY_LINE_REGEX, block[0]));
            assert!(matches(RUNTIME_LINE_REGEX, block[2]));
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
    }

    /// Runs the reporter from the start until the stop; 300 records of 5 us arrive at the first sleep.
    fn run_once(period_millis: i64, start: i64, stop_at: i64) -> Vec<(i64, String)> {
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
        run_reporter(
            "NetTest",
            period_millis,
            clock,
            || StatisticsSample {
                instances: totals(&[(records.get(), records.get() * 5000)]),
                connections: 0,
                runtime: None,
            },
            |text| printed.push((fake.now.get(), text.to_owned())),
        );
        printed
    }

    #[test]
    fn a_stop_before_the_first_tick_prints_the_whole_run() {
        let printed = run_once(10_000, 2000 * SECOND, 2000 * SECOND + SECOND * 3 / 2);
        assert_eq!(printed.len(), 1);
        assert_eq!(printed[0].0, 2000 * SECOND + SECOND * 3 / 2);
        assert_eq!(
            lines(&printed[0].1)[0],
            "{*NetTestProducerSide-NetTest*} data 200 rps, lag 5 us, instances 1, connections 0"
        );
    }

    #[test]
    fn a_last_block_shorter_than_one_second_is_not_printed() {
        assert!(run_once(1000, 2000 * SECOND, 2000 * SECOND + SECOND / 2).is_empty());
        // A tick at 2005 primes, one at 2010 prints; 999 ms after it the stop prints nothing, 1 s after it a block.
        assert_eq!(run_once(5000, 2000 * SECOND, 2011 * SECOND - 1_000_000).len(), 1);
        assert_eq!(run_once(5000, 2000 * SECOND, 2011 * SECOND).len(), 2);
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
            "NetTest",
            1000,
            clock,
            || StatisticsSample {
                instances: totals(&[(0, 0)]),
                ..StatisticsSample::default()
            },
            |_| {},
        );
        assert_eq!(fake.targets.borrow()[..3], [101 * SECOND, 102 * SECOND, 103 * SECOND]);
    }

    #[test]
    fn the_system_clock_prints_aligned_blocks_until_the_stop() {
        let stop = Stop::default();
        let mut printed = Vec::new();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                std::thread::sleep(Duration::from_millis(2_600));
                stop.request();
            });
            run_reporter(
                "NetTest",
                500,
                system_clock(&stop),
                || StatisticsSample {
                    instances: totals(&[(0, 0)]),
                    ..StatisticsSample::default()
                },
                |text| printed.push(text.to_owned()),
            );
        });
        // Ticks every 500 ms for about 2.6 s: the first primes, then 3 to 5 blocks.
        assert!((2..=6).contains(&printed.len()), "{printed:?}");
    }
}
