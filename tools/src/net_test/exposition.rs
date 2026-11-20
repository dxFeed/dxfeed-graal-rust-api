//! The `/metrics` text of a NetTest side: the Prometheus text exposition format 0.0.4 (section "Metrics" of the
//! specification), formatted from a snapshot of plain values. The counters live in [`LiveMetrics`]; the generators and
//! the listeners update their atomics, the reporter thread stores the values that need a call into the isolate once
//! per interval, and the HTTP thread only reads, so a scrape never calls into the isolate.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use super::cli::{IMPLEMENTATION, Mode, Options, SPEC_VERSION, role_name};
use super::consumer::{LAG_BUCKET_BOUNDS_NANOS, LiveLags};
use super::generator::{Counters, PublishCounters};
use super::limiter::lock_ignoring_poison;
use super::statistics::RuntimeSample;
use super::summary::ConnectorTotals;

/// The content type of `/metrics`.
pub const CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// The buckets of the lag histogram: the bounds and `+Inf`.
pub const LAG_BUCKETS: usize = LAG_BUCKET_BOUNDS_NANOS.len() + 1;
/// The bounds of the buckets as the sample of the kit writes them.
const LAG_BUCKET_TEXTS: [&str; LAG_BUCKETS] = [
    "5e-05", "0.0001", "0.00025", "0.0005", "0.001", "0.0025", "0.005", "0.01", "0.025", "0.05", "0.1", "0.25", "1",
    "+Inf",
];

/// The labels and the configuration of the series.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetricsInfo {
    pub producer: bool,
    pub name: String,
    pub version: String,
    /// Empty when the isolate did not give the version.
    pub sdk_version: String,
    pub role: String,
    pub event: String,
    /// Empty without `-c`.
    pub collector: String,
    /// `p` with `-r`: the schedule slip is exported.
    pub scheduled: bool,
}

/// The e2e lags of a consumer at one moment.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LagSnapshot {
    /// The lags per bucket, not cumulative, `+Inf` last.
    pub buckets: [i64; LAG_BUCKETS],
    pub count: i64,
    /// The sum of the non-negative lags in nanoseconds.
    pub positive_sum_nanos: i64,
    pub negative: i64,
}

/// Everything `/metrics` shows, as plain values.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    /// The records per instance.
    pub records: Vec<i64>,
    pub publish_nanos: i64,
    pub publish_calls: i64,
    /// `None` without a schedule.
    pub slip_last_nanos: Option<i64>,
    pub lags: LagSnapshot,
    /// The open connections per instance.
    pub connections: Vec<i64>,
    pub connectors: ConnectorTotals,
    pub runtime: RuntimeSample,
}

/// Nanoseconds as seconds in decimal without an exponent or trailing zeros: `86100000000` is `86.1`, `12000` is
/// `0.000012`.
pub fn format_seconds(nanos: i64) -> String {
    let magnitude = nanos.unsigned_abs();
    let fraction = format!("{:09}", magnitude % 1_000_000_000);
    let fraction = fraction.trim_end_matches('0');
    let sign = if nanos < 0 { "-" } else { "" };
    let whole = magnitude / 1_000_000_000;
    if fraction.is_empty() {
        format!("{sign}{whole}")
    } else {
        format!("{sign}{whole}.{fraction}")
    }
}

/// The value of a label with `\`, `"` and the line break escaped.
fn label_value(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for symbol in value.chars() {
        match symbol {
            '\\' | '"' => {
                escaped.push('\\');
                escaped.push(symbol);
            }
            '\n' => escaped.push_str("\\n"),
            _ => escaped.push(symbol),
        }
    }
    escaped
}

/// The text under construction with the labels every series has.
struct Text {
    text: String,
    labels: String,
}

impl Text {
    fn head(&mut self, metric: &str, kind: &str, help: &str) {
        self.text
            .push_str(&format!("# HELP {metric} {help}\n# TYPE {metric} {kind}\n"));
    }

    /// A series with the common labels and the extra ones (`,key="value"`).
    fn series(&mut self, metric: &str, value: &str, extra: &str) {
        self.text
            .push_str(&format!("{metric}{{{}{extra}}} {value}\n", self.labels));
    }

    /// A metric of one series; an absent value keeps the HELP and TYPE lines and leaves the series out.
    fn single(&mut self, metric: &str, kind: &str, help: &str, value: Option<String>) {
        self.head(metric, kind, help);
        if let Some(value) = value {
            self.series(metric, &value, "");
        }
    }

    /// One series per instance with the label `connection`.
    fn per_connection(&mut self, metric: &str, kind: &str, help: &str, values: &[i64]) {
        self.head(metric, kind, help);
        for (index, value) in values.iter().enumerate() {
            self.series(metric, &value.to_string(), &format!(",connection=\"{}\"", index + 1));
        }
    }
}

fn count(value: i64) -> Option<String> {
    Some(value.to_string())
}

fn seconds(nanos: i64) -> Option<String> {
    Some(format_seconds(nanos))
}

/// The text of `/metrics`: the metrics of the side with their HELP and TYPE lines, in the order of the sample of the
/// kit. Without v2 statistics the three v2 families are absent.
pub fn format_metrics(info: &MetricsInfo, snapshot: &Snapshot) -> String {
    let side = if info.producer { "producer" } else { "consumer" };
    let mut text = Text {
        text: String::new(),
        labels: format!(
            "tool=\"{IMPLEMENTATION}\",side=\"{side}\",name=\"{}\"",
            label_value(&info.name)
        ),
    };
    text.head("nettest_info", "gauge", "The configuration of the NetTest run.");
    text.series(
        "nettest_info",
        "1",
        &format!(
            ",spec_version=\"{SPEC_VERSION}\",version=\"{}\",sdk_version=\"{}\",role=\"{}\",event=\"{}\",\
             collector=\"{}\"",
            label_value(&info.version),
            label_value(&info.sdk_version),
            label_value(&info.role),
            label_value(&info.event),
            label_value(&info.collector),
        ),
    );
    text.per_connection(
        "nettest_records_total",
        "counter",
        if info.producer {
            "The published events."
        } else {
            "The received events."
        },
        &snapshot.records,
    );
    if info.producer {
        text.single(
            "nettest_publish_seconds_total",
            "counter",
            "The time of the batch fills and the publishing calls.",
            seconds(snapshot.publish_nanos),
        );
        text.single(
            "nettest_publish_calls_total",
            "counter",
            "The publishing calls.",
            count(snapshot.publish_calls),
        );
        text.single(
            "nettest_schedule_slip_seconds",
            "gauge",
            "The schedule slip of the last batch.",
            snapshot.slip_last_nanos.and_then(seconds),
        );
    } else {
        let lags = &snapshot.lags;
        text.head(
            "nettest_e2e_lag_seconds",
            "histogram",
            "The receive time minus the time of the event.",
        );
        let mut cumulative = 0_i64;
        for (index, bucket) in lags.buckets.iter().enumerate() {
            cumulative = cumulative.saturating_add(*bucket);
            // +Inf is the count, so that the buckets never exceed it.
            let value = if index + 1 == LAG_BUCKETS {
                lags.count
            } else {
                cumulative.min(lags.count)
            };
            text.series(
                "nettest_e2e_lag_seconds_bucket",
                &value.to_string(),
                &format!(",le=\"{}\"", LAG_BUCKET_TEXTS[index]),
            );
        }
        text.series(
            "nettest_e2e_lag_seconds_sum",
            &format_seconds(lags.positive_sum_nanos),
            "",
        );
        text.series("nettest_e2e_lag_seconds_count", &lags.count.to_string(), "");
        text.single(
            "nettest_e2e_lag_negative_total",
            "counter",
            "The events received with a negative lag.",
            count(lags.negative),
        );
    }
    text.per_connection(
        "nettest_connections",
        "gauge",
        "The open connections.",
        &snapshot.connections,
    );
    let connectors = &snapshot.connectors;
    text.single(
        "nettest_connector_read_bytes_total",
        "counter",
        "The bytes read by the connectors.",
        count(connectors.read_bytes),
    );
    text.single(
        "nettest_connector_written_bytes_total",
        "counter",
        "The bytes written by the connectors.",
        count(connectors.written_bytes),
    );
    text.single(
        "nettest_connector_read_records_total",
        "counter",
        "The data records read by the connectors.",
        count(connectors.read_records),
    );
    text.single(
        "nettest_connector_written_records_total",
        "counter",
        "The data records written by the connectors.",
        count(connectors.written_records),
    );
    // The publisher has no time marks on its path, so its connector lag means nothing and is not exported.
    if let Some(v2) = connectors.v2 {
        if !info.producer {
            text.single(
                "nettest_connector_lag_seconds_total",
                "counter",
                "The sum of the connector lags of the data records read.",
                seconds(v2.read_lag_us.saturating_mul(1000)),
            );
        }
        text.single(
            "nettest_connector_rtt_seconds_total",
            "counter",
            "The sum of the round-trip times of the records read and written by the connectors.",
            seconds(v2.rtt_us.saturating_mul(1000)),
        );
        text.single(
            "nettest_connector_sub_records_total",
            "counter",
            "The subscription records read and written by the connectors.",
            count(v2.sub_records),
        );
    }
    let runtime = &snapshot.runtime;
    let both = |first: Option<i64>, second: Option<i64>| Some(first? + second?);
    text.single(
        "nettest_runtime_heap_used_bytes",
        "gauge",
        "The used heap.",
        runtime.heap_used_bytes.and_then(count),
    );
    text.single(
        "nettest_runtime_heap_committed_bytes",
        "gauge",
        "The committed heap.",
        runtime.heap_committed_bytes.and_then(count),
    );
    text.single(
        "nettest_runtime_gc_total",
        "counter",
        "The collections.",
        both(runtime.young_gc_count, runtime.full_gc_count).and_then(count),
    );
    text.single(
        "nettest_runtime_gc_seconds_total",
        "counter",
        "The time of the collections.",
        both(runtime.young_gc_time_ms, runtime.full_gc_time_ms)
            .and_then(|millis| seconds(millis.saturating_mul(1_000_000))),
    );
    text.single(
        "nettest_runtime_cpu_seconds_total",
        "counter",
        "The CPU time of the process.",
        runtime.process_cpu_time_ns.and_then(seconds),
    );
    text.text
}

/// What the reporter thread stores once per interval: the values that take a call into the isolate.
#[derive(Debug, Default)]
struct Cached {
    connections: Vec<i64>,
    connectors: ConnectorTotals,
    runtime: RuntimeSample,
}

/// The counters of a run that `/metrics` shows. The generators and the listeners write the shared atomics live; the
/// reporter thread calls [`LiveMetrics::store`] once per interval with the values it read from the isolate.
pub struct LiveMetrics {
    info: MetricsInfo,
    /// The records per instance, the counters of the statistics lines.
    pub records: Vec<Arc<Counters>>,
    /// `p`: the publishing calls of all generators.
    pub publish: PublishCounters,
    /// `c`: the e2e lags of all listeners.
    pub lags: Arc<LiveLags>,
    cached: Mutex<Cached>,
}

impl LiveMetrics {
    pub fn new(info: MetricsInfo, instances: usize) -> Self {
        LiveMetrics {
            info,
            records: (0..instances).map(|_| Arc::new(Counters::default())).collect(),
            publish: PublishCounters::default(),
            lags: Arc::default(),
            cached: Mutex::new(Cached {
                connections: vec![0; instances],
                ..Cached::default()
            }),
        }
    }

    /// The metrics of the run of the options; `sdk_version` is `None` when the isolate did not give it.
    pub fn for_run(options: &Options, sdk_version: Option<&str>) -> Self {
        let info = MetricsInfo {
            producer: options.mode == Mode::Producer,
            name: options.name.clone(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            sdk_version: sdk_version.unwrap_or_default().to_owned(),
            role: role_name(options.role()).to_owned(),
            event: options.event.name().to_owned(),
            collector: options.collector.map_or_else(String::new, |c| c.name().to_owned()),
            scheduled: options.mode == Mode::Producer && options.records_per_second.is_some(),
        };
        LiveMetrics::new(info, options.instances)
    }

    /// Keeps the values of the last tick of the reporter: the open connections per instance, the connector statistics
    /// and the runtime values.
    pub fn store(&self, connections: Vec<i64>, connectors: ConnectorTotals, runtime: RuntimeSample) {
        *lock_ignoring_poison(&self.cached) = Cached {
            connections,
            connectors,
            runtime,
        };
    }

    /// The values now. The lag count is read first and the buckets after it: a scrape that sees a count sees the
    /// buckets of at least the same records.
    pub fn snapshot(&self) -> Snapshot {
        let lags = &*self.lags;
        let count = lags.count.load(Ordering::Acquire);
        let mut buckets = [0_i64; LAG_BUCKETS];
        for (value, live) in buckets.iter_mut().zip(&lags.buckets) {
            *value = live.load(Ordering::Relaxed);
        }
        let cached = lock_ignoring_poison(&self.cached);
        let publish = &self.publish;
        Snapshot {
            records: self
                .records
                .iter()
                .map(|counters| counters.records.load(Ordering::Relaxed))
                .collect(),
            publish_nanos: publish.nanos.load(Ordering::Relaxed),
            publish_calls: publish.calls.load(Ordering::Relaxed),
            slip_last_nanos: self
                .info
                .scheduled
                .then(|| publish.slip_last_nanos.load(Ordering::Relaxed)),
            lags: LagSnapshot {
                buckets,
                count,
                positive_sum_nanos: lags.positive_sum_nanos.load(Ordering::Relaxed),
                negative: lags.negative.load(Ordering::Relaxed),
            },
            connections: cached.connections.clone(),
            connectors: cached.connectors,
            runtime: cached.runtime,
        }
    }

    /// The text of `/metrics` now.
    pub fn render(&self) -> String {
        format_metrics(&self.info, &self.snapshot())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use super::super::consumer::lag_bucket_index;
    use super::super::kit::kit_text;
    use super::super::summary::ConnectorTotalsV2;
    use super::*;

    fn info(producer: bool, name: &str, role: &str, scheduled: bool) -> MetricsInfo {
        MetricsInfo {
            producer,
            name: name.to_owned(),
            version: "3.18.0".to_owned(),
            sdk_version: "3.18.0".to_owned(),
            role: role.to_owned(),
            event: "Trade".to_owned(),
            collector: "stream".to_owned(),
            scheduled,
        }
    }

    fn runtime(heap: i64, committed: i64, gc: i64, gc_ms: i64, cpu_ns: i64) -> RuntimeSample {
        RuntimeSample {
            heap_used_bytes: Some(heap),
            heap_committed_bytes: Some(committed),
            young_gc_count: Some(gc),
            young_gc_time_ms: Some(gc_ms),
            full_gc_count: Some(0),
            full_gc_time_ms: Some(0),
            process_cpu_time_ns: Some(cpu_ns),
        }
    }

    /// A sample of the kit with the tool of the series replaced; the line breaks are LF whatever the checkout did to
    /// the file.
    fn kit_sample(name: &str, tool: &str) -> String {
        kit_text(name)
            .replace("\r\n", "\n")
            .replace(&format!("tool=\"{tool}\""), "tool=\"rust-api\"")
    }

    /// The first difference of two texts, for the message of a failure.
    fn first_difference(actual: &str, expected: &str) -> String {
        let (actual_lines, expected_lines): (Vec<&str>, Vec<&str>) =
            (actual.lines().collect(), expected.lines().collect());
        for index in 0..actual_lines.len().max(expected_lines.len()) {
            let (a, e) = (actual_lines.get(index), expected_lines.get(index));
            if a != e {
                return format!("line {}: got {a:?}, expected {e:?}", index + 1);
            }
        }
        "the same lines".to_owned()
    }

    /// The sample of the consumer in the cumulative buckets of the text, as the counts per bucket.
    fn consumer_lags() -> LagSnapshot {
        let cumulative = [
            0,
            1_200_000,
            42_000_000,
            150_000_000,
            200_500_000,
            208_000_000,
            209_500_000,
            209_900_000,
            209_990_000,
            209_998_000,
            209_999_000,
            209_999_000,
            209_999_000,
            209_999_000,
        ];
        let mut buckets = [0_i64; LAG_BUCKETS];
        for index in 0..LAG_BUCKETS {
            buckets[index] = cumulative[index] - if index == 0 { 0 } else { cumulative[index - 1] };
        }
        LagSnapshot {
            buckets,
            count: 209_999_000,
            positive_sum_nanos: 85_049_595_000_000,
            negative: 0,
        }
    }

    fn producer_snapshot() -> Snapshot {
        Snapshot {
            records: vec![210_000_000],
            publish_nanos: 86_100_000_000,
            publish_calls: 2_100_000,
            slip_last_nanos: Some(12_000),
            lags: LagSnapshot::default(),
            connections: vec![1],
            connectors: ConnectorTotals {
                read_bytes: 1200,
                written_bytes: 6_825_000_000,
                read_records: 0,
                written_records: 210_000_000,
                // The lag is not exported on the producer, so its value does not matter.
                v2: Some(ConnectorTotalsV2 {
                    sub_records: 100_000,
                    read_lag_us: 0,
                    rtt_us: 42_020_000_000,
                }),
            },
            runtime: runtime(125_829_120, 268_435_456, 312, 1520, 402_700_000_000),
        }
    }

    fn consumer_snapshot() -> Snapshot {
        Snapshot {
            records: vec![105_000_000, 104_999_000],
            publish_nanos: 0,
            publish_calls: 0,
            slip_last_nanos: None,
            lags: consumer_lags(),
            connections: vec![1, 1],
            connectors: ConnectorTotals {
                read_bytes: 6_825_000_000,
                written_bytes: 1200,
                read_records: 209_999_000,
                written_records: 0,
                v2: Some(ConnectorTotalsV2 {
                    sub_records: 200_000,
                    read_lag_us: 31_499_850_000,
                    rtt_us: 42_039_800_000,
                }),
            },
            runtime: runtime(325_058_560, 1_073_741_824, 41, 380, 365_400_000_000),
        }
    }

    #[test]
    fn the_producer_text_is_the_sample_of_the_kit_byte_for_byte() {
        let actual = format_metrics(&info(true, "bench", "STREAM_PUBLISHER", true), &producer_snapshot());
        let expected = kit_sample("metrics-sample-producer.txt", "c-api");
        assert_eq!(actual, expected, "{}", first_difference(&actual, &expected));
    }

    #[test]
    fn the_consumer_text_is_the_sample_of_the_kit_byte_for_byte() {
        let actual = format_metrics(&info(false, "bench", "STREAM_FEED", false), &consumer_snapshot());
        let expected = kit_sample("metrics-sample-consumer.txt", "java-api");
        assert_eq!(actual, expected, "{}", first_difference(&actual, &expected));
    }

    #[test]
    fn live_counters_with_the_sample_values_give_the_consumer_sample() {
        let live = LiveMetrics::new(info(false, "bench", "STREAM_FEED", false), 2);
        let sample = consumer_snapshot();
        live.records[0].records.store(sample.records[0], Ordering::Relaxed);
        live.records[1].records.store(sample.records[1], Ordering::Relaxed);
        for (live, value) in live.lags.buckets.iter().zip(sample.lags.buckets) {
            live.store(value, Ordering::Relaxed);
        }
        live.lags
            .positive_sum_nanos
            .store(sample.lags.positive_sum_nanos, Ordering::Relaxed);
        live.lags.count.store(sample.lags.count, Ordering::Release);
        live.store(sample.connections.clone(), sample.connectors, sample.runtime);
        let expected = kit_sample("metrics-sample-consumer.txt", "java-api");
        assert_eq!(live.render(), expected);
    }

    #[test]
    fn live_counters_with_the_sample_values_give_the_producer_sample() {
        let live = LiveMetrics::new(info(true, "bench", "STREAM_PUBLISHER", true), 1);
        let sample = producer_snapshot();
        live.records[0].records.store(sample.records[0], Ordering::Relaxed);
        live.publish.nanos.store(sample.publish_nanos, Ordering::Relaxed);
        live.publish.calls.store(sample.publish_calls, Ordering::Relaxed);
        live.publish.slip_last_nanos.store(12_000, Ordering::Relaxed);
        live.store(sample.connections.clone(), sample.connectors, sample.runtime);
        let expected = kit_sample("metrics-sample-producer.txt", "c-api");
        assert_eq!(live.render(), expected);
    }

    #[test]
    fn a_value_that_the_tool_does_not_have_gets_no_series_but_keeps_its_head() {
        let live = LiveMetrics::new(info(true, "NetTest", "PUBLISHER", false), 1);
        let text = live.render();
        assert!(text.contains("nettest_records_total{"), "{text}");
        for metric in [
            "nettest_schedule_slip_seconds",
            "nettest_runtime_heap_used_bytes",
            "nettest_runtime_gc_seconds_total",
            "nettest_runtime_cpu_seconds_total",
        ] {
            assert!(text.contains(&format!("# TYPE {metric} ")), "{metric}\n{text}");
            assert!(!text.contains(&format!("{metric}{{")), "{metric}\n{text}");
        }
    }

    #[test]
    fn without_the_statistics_v2_their_families_are_absent_and_those_of_v1_stay() {
        for producer in [true, false] {
            let mut snapshot = if producer {
                producer_snapshot()
            } else {
                consumer_snapshot()
            };
            snapshot.connectors = ConnectorTotals {
                read_bytes: 10,
                written_bytes: 20,
                read_records: 30,
                written_records: 40,
                v2: None,
            };
            let side = if producer { "producer" } else { "consumer" };
            let text = format_metrics(&info(producer, "NetTest", "STREAM_FEED", false), &snapshot);
            assert!(
                text.contains("# TYPE nettest_connector_read_bytes_total counter\n"),
                "{text}"
            );
            assert!(
                text.contains(&format!(
                    "nettest_connector_written_records_total{{tool=\"rust-api\",side=\"{side}\",name=\"NetTest\"}} 40\n"
                )),
                "{text}"
            );
            for absent in [
                "nettest_connector_lag_seconds_total",
                "nettest_connector_rtt_seconds_total",
                "nettest_connector_sub_records_total",
            ] {
                assert!(!text.contains(absent), "{absent}\n{text}");
            }
        }
    }

    #[test]
    fn the_statistics_v2_are_counters_in_seconds_and_records_and_the_lag_is_the_consumer_only() {
        for producer in [true, false] {
            let mut snapshot = if producer {
                producer_snapshot()
            } else {
                consumer_snapshot()
            };
            snapshot.connectors.v2 = Some(ConnectorTotalsV2 {
                sub_records: 7,
                read_lag_us: 1_500_001,
                rtt_us: 2_000_000,
            });
            let side = if producer { "producer" } else { "consumer" };
            let text = format_metrics(&info(producer, "NetTest", "STREAM_FEED", false), &snapshot);
            let labels = format!("{{tool=\"rust-api\",side=\"{side}\",name=\"NetTest\"}}");
            assert!(
                text.contains(&format!(
                    "# TYPE nettest_connector_rtt_seconds_total counter\nnettest_connector_rtt_seconds_total{labels} 2\n"
                )),
                "{text}"
            );
            assert!(
                text.contains(&format!(
                    "# TYPE nettest_connector_sub_records_total counter\nnettest_connector_sub_records_total{labels} 7\n"
                )),
                "{text}"
            );
            let lag = format!(
                "# TYPE nettest_connector_lag_seconds_total counter\nnettest_connector_lag_seconds_total{labels} 1.500001\n"
            );
            assert_eq!(text.contains(&lag), !producer, "{text}");
            assert_eq!(
                text.contains("nettest_connector_lag_seconds_total"),
                !producer,
                "{text}"
            );
        }
    }

    #[test]
    fn the_lags_fall_into_the_buckets_at_the_bounds_and_a_negative_one_into_the_first() {
        assert_eq!(lag_bucket_index(-1), 0);
        assert_eq!(lag_bucket_index(0), 0);
        assert_eq!(lag_bucket_index(50_000), 0);
        assert_eq!(lag_bucket_index(50_001), 1);
        assert_eq!(lag_bucket_index(1_000_000_000), LAG_BUCKET_BOUNDS_NANOS.len() - 1);
        assert_eq!(lag_bucket_index(1_000_000_001), LAG_BUCKET_BOUNDS_NANOS.len());
        assert_eq!(LAG_BUCKET_TEXTS.len(), LAG_BUCKET_BOUNDS_NANOS.len() + 1);
    }

    #[test]
    fn the_bounds_of_the_buckets_read_as_the_numbers_of_the_bounds() {
        for (text, bound) in LAG_BUCKET_TEXTS.iter().zip(LAG_BUCKET_BOUNDS_NANOS) {
            let seconds: f64 = text.parse().unwrap();
            assert_eq!((seconds * 1e9).round() as i64, bound, "{text}");
        }
        assert_eq!(LAG_BUCKET_TEXTS[LAG_BUCKETS - 1], "+Inf");
    }

    #[test]
    fn seconds_are_decimal_without_an_exponent_or_trailing_zeros() {
        assert_eq!(format_seconds(86_100_000_000), "86.1");
        assert_eq!(format_seconds(12_000), "0.000012");
        assert_eq!(format_seconds(0), "0");
        assert_eq!(format_seconds(1), "0.000000001");
        assert_eq!(format_seconds(1_000_000_000), "1");
        assert_eq!(format_seconds(-1_500_000_000), "-1.5");
        assert_eq!(format_seconds(i64::MIN), "-9223372036.854775808");
    }

    #[test]
    fn the_counts_never_exceed_the_total_and_inf_is_the_total() {
        // A scrape between the buckets and the count of one listener call: the buckets are ahead of the count.
        let mut snapshot = consumer_snapshot();
        snapshot.lags.buckets[0] = 10;
        snapshot.lags.count = 4;
        let text = format_metrics(&info(false, "NetTest", "STREAM_FEED", false), &snapshot);
        assert!(text.contains("le=\"5e-05\"} 4\n"), "{text}");
        assert!(text.contains("le=\"+Inf\"} 4\n"), "{text}");
        assert!(
            text.contains("nettest_e2e_lag_seconds_count{tool=\"rust-api\",side=\"consumer\",name=\"NetTest\"} 4\n")
        );
    }

    #[test]
    fn label_values_are_escaped() {
        let snapshot = Snapshot {
            records: vec![1],
            connections: vec![0],
            ..Snapshot::default()
        };
        let text = format_metrics(&info(true, "a\"b\\c\nd", "PUBLISHER", false), &snapshot);
        assert!(text.contains("name=\"a\\\"b\\\\c\\nd\""), "{text}");
    }
}
