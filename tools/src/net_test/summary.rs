//! The summary file of NetTest, `nettest-summary/1`: the model of a finished run, the writer of its JSON in the exact
//! layout of the C tool, and the write through `<file>.tmp` and a rename.
//!
//! The layout is the one of the C tool: the top level is one `"key": value` per line with two spaces of indent, an
//! object is on one line with `, ` between its members, and the arrays `intervals` and `connectors` have one element
//! per line with four spaces of indent. No JSON library is needed for it: only strings need an escape.

use std::fs;
use std::path::PathBuf;

use dxfeed_graal::{DXEndpoint, MessageConnector, system};

use super::Status;
use super::cli::{IMPLEMENTATION, Mode, Options, SPEC_VERSION, role_name};
use super::generator::PublishCounters;
use super::histogram::{HISTOGRAM_DIGITS, HISTOGRAM_HIGHEST, HISTOGRAM_LOWEST};
use super::statistics::{
    BYTES_PER_MB, E2eTotals, IntervalRecord, ReporterOutcome, RuntimeSample, TimedRuntime, records_per_second,
};

/// The name of the tool in the summary.
const TOOL_NAME: &str = "NetTest";

/// The e2e lag totals of the measured intervals of a consumer. With no lag the values are `None`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LagSummary {
    pub count: i64,
    pub negative: i64,
    pub p50_ns: Option<i64>,
    pub p90_ns: Option<i64>,
    pub p99_ns: Option<i64>,
    pub p999_ns: Option<i64>,
    pub p9999_ns: Option<i64>,
    pub max_ns: Option<i64>,
    pub mean_ns: Option<i64>,
}

/// The publishing calls of a producer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PublishSummary {
    pub calls: i64,
    pub events: i64,
    /// The nanoseconds of the fills and the publishing calls.
    pub nanos: i64,
}

/// The slip of the schedule of a producer with `-r`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SlipSummary {
    pub max_ns: i64,
    pub end_ns: i64,
    pub growth_ns_per_s: Option<i64>,
}

/// The values of the isolate over the run: the heap at the end and the growth of the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RuntimeSummary {
    pub heap_used_mb: Option<i64>,
    pub heap_committed_mb: Option<i64>,
    pub gc_count: Option<i64>,
    pub gc_time_ms: Option<i64>,
    pub cpu_percent_mean: Option<i64>,
}

/// The statistics v1 of a connector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ConnectorV1 {
    pub read_bytes: i64,
    pub written_bytes: i64,
    pub read_data_records: i64,
    pub written_data_records: i64,
    pub active_connections: i64,
    pub closed_connections: i64,
}

/// What the statistics v2 add to v1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ConnectorV2 {
    pub read_sub_records: i64,
    pub written_sub_records: i64,
    pub data_read_lag_us: i64,
    pub data_write_lag_us: i64,
    pub read_rtt_us: i64,
    pub write_rtt_us: i64,
}

/// A connector with its statistics, read once at the end of the run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectorSummary {
    pub address: String,
    pub v1: ConnectorV1,
    /// `None` when the SDK has no statistics v2.
    pub v2: Option<ConnectorV2>,
}

/// What the summary reports of a run.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RunReport {
    /// The wall-clock time in nanoseconds since the Unix epoch where the run started and where its reporter stopped.
    pub start_epoch_nanos: i64,
    pub end_epoch_nanos: i64,
    pub intervals: Vec<IntervalRecord>,
    /// `c`: the e2e lag of the measured intervals.
    pub e2e_lag: Option<LagSummary>,
    /// `p`: the publishing calls.
    pub publish: Option<PublishSummary>,
    /// `p` with `-r`: the slip of the schedule.
    pub slip: Option<SlipSummary>,
    /// `c`: the symbols that got at least one event.
    pub symbols_seen: Option<i64>,
    pub runtime: RuntimeSummary,
    pub connectors: Vec<ConnectorSummary>,
}

/// How a run ended and what it reported: the data of the summary file and of the exit code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunResult {
    pub status: Status,
    /// The text after `NetTest: ` on stderr for a failed run; `None` for `ok` and a signal.
    pub error: Option<String>,
    pub report: RunReport,
}

impl RunReport {
    /// A report of a run that has nothing to report: only the two times are set.
    pub fn at(start_epoch_nanos: i64, end_epoch_nanos: i64) -> Self {
        RunReport {
            start_epoch_nanos,
            end_epoch_nanos,
            ..RunReport::default()
        }
    }
}

impl RunResult {
    /// A run that failed before it had anything to report.
    pub fn failed(message: String, start_epoch_nanos: i64, end_epoch_nanos: i64) -> Self {
        RunResult {
            status: Status::Error,
            error: Some(message),
            report: RunReport::at(start_epoch_nanos, end_epoch_nanos),
        }
    }
}

/// The summary of the e2e lag totals of the measured intervals.
pub fn lag_summary(totals: &E2eTotals) -> LagSummary {
    let stats = totals.stats;
    let mut lag = LagSummary {
        count: stats.count,
        negative: stats.negative,
        ..LagSummary::default()
    };
    if stats.count > 0 {
        let percentile = |percentile: f64| Some(totals.histogram.value_at_percentile(percentile));
        lag.p50_ns = percentile(50.0);
        lag.p90_ns = percentile(90.0);
        lag.p99_ns = percentile(99.0);
        lag.p999_ns = percentile(99.9);
        lag.p9999_ns = percentile(99.99);
        lag.max_ns = Some(stats.max);
        // The sum of the signed lags over their count, divided toward zero.
        lag.mean_ns = Some(stats.sum / stats.count);
    }
    lag
}

/// The slip of the schedule: the largest of the measured intervals (of all when none is measured), the slip of the
/// last batch and the growth per second between the ends of the first and the last measured interval.
pub fn slip_summary(intervals: &[IntervalRecord], last_slip_nanos: i64) -> SlipSummary {
    let measured: Vec<&IntervalRecord> = intervals.iter().filter(|interval| !interval.warmup).collect();
    let max_ns = intervals
        .iter()
        .filter(|interval| measured.is_empty() || !interval.warmup)
        .filter_map(|interval| interval.slip_max_ns)
        .fold(0, i64::max);
    let growth_ns_per_s = match (measured.first(), measured.last()) {
        (Some(first), Some(last)) if measured.len() >= 2 => {
            let millis = last.end_epoch_ms() - first.end_epoch_ms();
            match (first.slip_end_ns, last.slip_end_ns) {
                (Some(first_slip), Some(last_slip)) if millis > 0 => Some((last_slip - first_slip) * 1000 / millis),
                _ => None,
            }
        }
        _ => None,
    };
    SlipSummary {
        max_ns,
        end_ns: last_slip_nanos,
        growth_ns_per_s,
    }
}

/// The runtime of the run: the heap of the last sample, the growth of the collections and of the CPU time between the
/// first and the last sample. A value that the isolate did not give, or that went back, stays `None`.
pub fn runtime_summary(first: &TimedRuntime, last: &TimedRuntime) -> RuntimeSummary {
    let megabytes = |bytes: Option<i64>| bytes.filter(|&bytes| bytes >= 0).map(|bytes| bytes / BYTES_PER_MB);
    let growth = |from: Option<i64>, to: Option<i64>| {
        let (from, to) = (from?, to?);
        (from >= 0 && to >= from).then_some(to - from)
    };
    let both = |one: Option<i64>, other: Option<i64>| Some(one? + other?);
    let (first_sample, last_sample): (&RuntimeSample, &RuntimeSample) = (&first.sample, &last.sample);
    let cpu = growth(first_sample.process_cpu_time_ns, last_sample.process_cpu_time_ns);
    let period = last.epoch_nanos - first.epoch_nanos;
    RuntimeSummary {
        heap_used_mb: megabytes(last_sample.heap_used_bytes),
        heap_committed_mb: megabytes(last_sample.heap_committed_bytes),
        gc_count: both(
            growth(first_sample.young_gc_count, last_sample.young_gc_count),
            growth(first_sample.full_gc_count, last_sample.full_gc_count),
        ),
        gc_time_ms: both(
            growth(first_sample.young_gc_time_ms, last_sample.young_gc_time_ms),
            growth(first_sample.full_gc_time_ms, last_sample.full_gc_time_ms),
        ),
        cpu_percent_mean: cpu.filter(|_| period > 0).map(|cpu| cpu.saturating_mul(100) / period),
    }
}

/// The part of the report that both sides share: the times, the intervals and the runtime.
fn common_report(start_epoch_nanos: i64, end_epoch_nanos: i64, reporter: ReporterOutcome) -> RunReport {
    let runtime = match (&reporter.first_runtime, &reporter.last_runtime) {
        (Some(first), Some(last)) => runtime_summary(first, last),
        _ => RuntimeSummary::default(),
    };
    RunReport {
        start_epoch_nanos,
        end_epoch_nanos,
        intervals: reporter.intervals,
        runtime,
        ..RunReport::default()
    }
}

/// The report of a consumer run.
pub fn consumer_report(
    start_epoch_nanos: i64,
    end_epoch_nanos: i64,
    reporter: ReporterOutcome,
    symbols_seen: i64,
    connectors: Vec<ConnectorSummary>,
) -> RunReport {
    let e2e_lag = Some(lag_summary(&reporter.e2e));
    RunReport {
        e2e_lag,
        symbols_seen: Some(symbols_seen),
        connectors,
        ..common_report(start_epoch_nanos, end_epoch_nanos, reporter)
    }
}

/// What a producer run counted besides its intervals.
#[derive(Debug, Clone, Copy)]
pub struct ProducerTotals<'a> {
    pub counters: &'a PublishCounters,
    /// The records of all instances.
    pub events: i64,
    /// Whether `-r` was given: only then the slip is reported.
    pub scheduled: bool,
}

/// The report of a producer run.
pub fn producer_report(
    start_epoch_nanos: i64,
    end_epoch_nanos: i64,
    reporter: ReporterOutcome,
    totals: ProducerTotals<'_>,
    connectors: Vec<ConnectorSummary>,
) -> RunReport {
    use std::sync::atomic::Ordering::Relaxed;
    let publish = PublishSummary {
        calls: totals.counters.calls.load(Relaxed),
        events: totals.events,
        nanos: totals.counters.nanos.load(Relaxed),
    };
    let report = common_report(start_epoch_nanos, end_epoch_nanos, reporter);
    let slip = totals
        .scheduled
        .then(|| slip_summary(&report.intervals, totals.counters.slip_last_nanos.load(Relaxed)));
    RunReport {
        publish: Some(publish),
        slip,
        connectors,
        ..report
    }
}

/// Whether the SDK reports the connector statistics v2.
pub fn io_statistics_supported() -> bool {
    system::is_feature_supported(system::feature::MESSAGE_CONNECTOR_IO_STATISTICS).unwrap_or(false)
}

/// The v2 totals of the connector statistics over all connectors.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConnectorTotalsV2 {
    /// The subscription records read and written.
    pub sub_records: i64,
    /// The sum of the lags of the data records read, in microseconds.
    pub read_lag_us: i64,
    /// The sum of the round-trip times of the records read and written, in microseconds.
    pub rtt_us: i64,
}

/// The connector statistics summed over the connectors of all instances; `v2` is `None` when the SDK has no v2.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConnectorTotals {
    pub read_bytes: i64,
    pub written_bytes: i64,
    pub read_records: i64,
    pub written_records: i64,
    pub v2: Option<ConnectorTotalsV2>,
}

/// The statistics of a connector: v2 (which has v1 too) when `io`, v1 otherwise; `None` when they cannot be read.
fn read_statistics(connector: &MessageConnector, io: bool) -> Option<(ConnectorV1, Option<ConnectorV2>)> {
    if io {
        let stats = connector.get_io_statistics().ok()?;
        let v1 = ConnectorV1 {
            read_bytes: stats.read_bytes,
            written_bytes: stats.written_bytes,
            read_data_records: stats.read_data_records,
            written_data_records: stats.written_data_records,
            active_connections: stats.active_connection_count,
            closed_connections: stats.closed_connection_count,
        };
        let v2 = ConnectorV2 {
            read_sub_records: stats.read_sub_records,
            written_sub_records: stats.written_sub_records,
            data_read_lag_us: stats.read_data_lag_sum_us,
            data_write_lag_us: stats.written_data_lag_sum_us,
            read_rtt_us: stats.read_rtt_sum_us,
            write_rtt_us: stats.written_rtt_sum_us,
        };
        Some((v1, Some(v2)))
    } else {
        let stats = connector.get_statistics().ok()?;
        let v1 = ConnectorV1 {
            read_bytes: stats.read_bytes,
            written_bytes: stats.written_bytes,
            read_data_records: stats.read_data_records,
            written_data_records: stats.written_data_records,
            active_connections: stats.active_connection_count,
            closed_connections: stats.closed_connection_count,
        };
        Some((v1, None))
    }
}

/// The statistics of the connectors of the endpoints, in order: v2 (which has v1 too) when the SDK reports the
/// feature, v1 otherwise. A connector whose statistics cannot be read is left out.
pub fn connector_summaries<'a>(endpoints: impl IntoIterator<Item = &'a DXEndpoint>) -> Vec<ConnectorSummary> {
    let io = io_statistics_supported();
    let mut summaries = Vec::new();
    for endpoint in endpoints {
        for connector in endpoint.get_connectors().unwrap_or_default() {
            let Some((v1, v2)) = read_statistics(&connector, io) else {
                continue;
            };
            summaries.push(ConnectorSummary {
                address: connector.get_address().unwrap_or_default(),
                v1,
                v2,
            });
        }
    }
    summaries
}

/// The statistics of the connectors of the endpoints summed, without their addresses; the v2 part is present when the
/// SDK reports the feature. A connector whose statistics cannot be read is left out.
pub fn connector_totals<'a>(endpoints: impl IntoIterator<Item = &'a DXEndpoint>) -> ConnectorTotals {
    let io = io_statistics_supported();
    let mut totals = ConnectorTotals {
        v2: io.then(ConnectorTotalsV2::default),
        ..ConnectorTotals::default()
    };
    for endpoint in endpoints {
        for connector in endpoint.get_connectors().unwrap_or_default() {
            let Some((v1, v2)) = read_statistics(&connector, io) else {
                continue;
            };
            totals.read_bytes += v1.read_bytes;
            totals.written_bytes += v1.written_bytes;
            totals.read_records += v1.read_data_records;
            totals.written_records += v1.written_data_records;
            if let (Some(sum), Some(v2)) = (totals.v2.as_mut(), v2) {
                sum.sub_records += v2.read_sub_records + v2.written_sub_records;
                sum.read_lag_us += v2.data_read_lag_us;
                sum.rtt_us += v2.read_rtt_us + v2.write_rtt_us;
            }
        }
    }
    totals
}

/// A JSON string with the quotes: `"`, `\`, the new line, the carriage return and the tab have the short escapes,
/// the other control characters `\u00xx`.
pub fn json_string(text: &str) -> String {
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('"');
    for symbol in text.chars() {
        match symbol {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            symbol if (symbol as u32) < 0x20 => quoted.push_str(&format!("\\u{:04x}", symbol as u32)),
            symbol => quoted.push(symbol),
        }
    }
    quoted.push('"');
    quoted
}

fn number(value: Option<i64>) -> String {
    value.map_or_else(|| "null".to_owned(), |value| value.to_string())
}

fn string_or_null(text: Option<&str>) -> String {
    text.map_or_else(|| "null".to_owned(), json_string)
}

fn string_array(values: &[String]) -> String {
    let items: Vec<String> = values.iter().map(|value| json_string(value)).collect();
    format!("[{}]", items.join(", "))
}

/// `"key": value` members joined into an object on one line.
struct Object(String);

impl Object {
    fn new() -> Self {
        Object("{".to_owned())
    }

    /// Adds a member; the value is JSON already.
    fn add(mut self, key: &str, value: impl AsRef<str>) -> Self {
        if self.0.len() > 1 {
            self.0.push_str(", ");
        }
        self.0.push_str(&format!("\"{key}\": {}", value.as_ref()));
        self
    }

    fn text(mut self) -> String {
        self.0.push('}');
        self.0
    }
}

fn config_of(options: &Options) -> String {
    let producer = options.mode == Mode::Producer;
    let producer_only = |value: String| if producer { value } else { "null".to_owned() };
    Object::new()
        .add("address", json_string(&options.address))
        .add("symbols", options.total_symbols.to_string())
        .add(
            "symbols_source",
            json_string(if options.symbols_file.is_some() {
                "file"
            } else {
                "generated"
            }),
        )
        .add("instances", options.instances.to_string())
        .add("rate_limit", producer_only(number(options.records_per_second)))
        .add("batch", producer_only(options.batch.to_string()))
        .add("event", json_string(options.event.name()))
        .add(
            "collector",
            options
                .collector
                .map_or_else(|| "null".to_owned(), |collector| json_string(collector.name())),
        )
        .add("role", json_string(role_name(options.role())))
        .add("reuse", producer_only(options.reuse.to_string()))
        // The Rust API copies the events into native structs on every call, so a reused batch is not reused below it.
        .add("reuse_effective", producer_only("false".to_owned()))
        .add("conflated", options.is_conflated().to_string())
        .add("period_ms", options.period_millis.to_string())
        .add("warmup_ms", number(options.warmup_millis))
        .add("duration_ms", number(options.duration_millis))
        .add("nano_time", options.is_nano_time_on().to_string())
        .add("isolate_arguments", string_array(&options.isolate_arguments))
        .text()
}

fn interval_of(interval: &IntervalRecord) -> String {
    Object::new()
        .add("start_epoch_ms", interval.start_epoch_ms().to_string())
        .add("end_epoch_ms", interval.end_epoch_ms().to_string())
        .add("phase", json_string(if interval.warmup { "warmup" } else { "measure" }))
        .add("records", interval.records.to_string())
        .add("rate", interval.rate.to_string())
        .add("lag_mean_ns", number(interval.lag_mean_ns))
        .add("lag_p50_ns", number(interval.lag_p50_ns))
        .add("lag_p99_ns", number(interval.lag_p99_ns))
        .add("lag_max_ns", number(interval.lag_max_ns))
        .add("publish_ns_per_event", number(interval.publish_ns_per_event))
        .add("slip_max_ns", number(interval.slip_max_ns))
        .add("slip_end_ns", number(interval.slip_end_ns))
        .add("connections", interval.connections.to_string())
        .text()
}

fn lag_of(lag: Option<&LagSummary>) -> String {
    let Some(lag) = lag else {
        return "null".to_owned();
    };
    let histogram = Object::new()
        .add("lowest", HISTOGRAM_LOWEST.to_string())
        .add("highest", HISTOGRAM_HIGHEST.to_string())
        .add("digits", HISTOGRAM_DIGITS.to_string())
        .text();
    Object::new()
        .add("count", lag.count.to_string())
        .add("negative", lag.negative.to_string())
        .add("p50_ns", number(lag.p50_ns))
        .add("p90_ns", number(lag.p90_ns))
        .add("p99_ns", number(lag.p99_ns))
        .add("p999_ns", number(lag.p999_ns))
        .add("p9999_ns", number(lag.p9999_ns))
        .add("max_ns", number(lag.max_ns))
        .add("mean_ns", number(lag.mean_ns))
        .add("histogram", histogram)
        .text()
}

fn publish_of(publish: Option<&PublishSummary>) -> String {
    let Some(publish) = publish else {
        return "null".to_owned();
    };
    Object::new()
        .add("calls", publish.calls.to_string())
        .add("events", publish.events.to_string())
        .add("ns_total", publish.nanos.to_string())
        .add(
            "ns_per_event_mean",
            number((publish.events > 0).then(|| publish.nanos / publish.events)),
        )
        .text()
}

fn slip_of(slip: Option<&SlipSummary>) -> String {
    let Some(slip) = slip else {
        return "null".to_owned();
    };
    Object::new()
        .add("max_ns", slip.max_ns.to_string())
        .add("end_ns", slip.end_ns.to_string())
        .add("growth_ns_per_s", number(slip.growth_ns_per_s))
        .text()
}

fn runtime_of(runtime: &RuntimeSummary) -> String {
    Object::new()
        .add("heap_used_mb", number(runtime.heap_used_mb))
        .add("heap_committed_mb", number(runtime.heap_committed_mb))
        .add("gc_count", number(runtime.gc_count))
        .add("gc_time_ms", number(runtime.gc_time_ms))
        .add("cpu_percent_mean", number(runtime.cpu_percent_mean))
        .text()
}

fn connector_of(connector: &ConnectorSummary) -> String {
    let v1 = &connector.v1;
    let v1 = Object::new()
        .add("read_bytes", v1.read_bytes.to_string())
        .add("written_bytes", v1.written_bytes.to_string())
        .add("read_data_records", v1.read_data_records.to_string())
        .add("written_data_records", v1.written_data_records.to_string())
        .add("active_connections", v1.active_connections.to_string())
        .add("closed_connections", v1.closed_connections.to_string())
        .text();
    let v2 = connector.v2.map_or_else(
        || "null".to_owned(),
        |v2| {
            Object::new()
                .add("read_sub_records", v2.read_sub_records.to_string())
                .add("written_sub_records", v2.written_sub_records.to_string())
                .add("data_read_lag_us", v2.data_read_lag_us.to_string())
                .add("data_write_lag_us", v2.data_write_lag_us.to_string())
                .add("read_rtt_us", v2.read_rtt_us.to_string())
                .add("write_rtt_us", v2.write_rtt_us.to_string())
                .text()
        },
    );
    Object::new()
        .add("address", json_string(&connector.address))
        .add("v1", v1)
        .add("v2", v2)
        .text()
}

/// An array with one element per line, `[]` when empty.
fn array_of<T>(items: &[T], format: impl Fn(&T) -> String) -> String {
    if items.is_empty() {
        return "[]".to_owned();
    }
    let lines: Vec<String> = items.iter().map(|item| format!("    {}", format(item))).collect();
    format!("[\n{}\n  ]", lines.join(",\n"))
}

/// The text of the summary file of a run. `sdk_version` is `None` when the isolate did not give it.
pub fn format_summary(options: &Options, sdk_version: Option<&str>, result: &RunResult) -> String {
    let report = &result.report;
    let build = Object::new()
        .add("vendor", "null")
        .add("march", "null")
        .add("gc", "null")
        .text();
    let tool = Object::new()
        .add("implementation", json_string(IMPLEMENTATION))
        .add("name", json_string(TOOL_NAME))
        .add("version", json_string(env!("CARGO_PKG_VERSION")))
        .add("sdk_version", string_or_null(sdk_version))
        .add("qd_version", "null")
        .add("build", build)
        .text();
    let records: i64 = report.intervals.iter().map(|interval| interval.records).sum();
    let rate = records_per_second(records, report.end_epoch_nanos - report.start_epoch_nanos);
    let side = if options.mode == Mode::Producer {
        "producer"
    } else {
        "consumer"
    };
    let members = [
        ("schema", json_string("nettest-summary/1")),
        ("spec_version", json_string(SPEC_VERSION)),
        ("tool", tool),
        ("side", json_string(side)),
        ("name", json_string(&options.name)),
        ("args", string_array(&options.arguments)),
        ("config", config_of(options)),
        ("start_epoch_ns", report.start_epoch_nanos.to_string()),
        ("end_epoch_ns", report.end_epoch_nanos.to_string()),
        ("status", json_string(result.status.name())),
        ("exit_code", result.status.exit_code().to_string()),
        (
            "error",
            string_or_null(result.error.as_deref().filter(|error| !error.is_empty())),
        ),
        ("records", records.to_string()),
        ("rate", rate.to_string()),
        ("time_source", json_string("actual")),
        (
            "time_resolution",
            json_string(if options.is_nano_time_on() { "ns" } else { "ms" }),
        ),
        ("intervals", array_of(&report.intervals, interval_of)),
        ("e2e_lag", lag_of(report.e2e_lag.as_ref())),
        ("publish_call", publish_of(report.publish.as_ref())),
        ("schedule_slip", slip_of(report.slip.as_ref())),
        ("symbols_seen", number(report.symbols_seen)),
        ("runtime", runtime_of(&report.runtime)),
        ("connectors", array_of(&report.connectors, connector_of)),
    ];
    let lines: Vec<String> = members
        .iter()
        .map(|(key, value)| format!("  \"{key}\": {value}"))
        .collect();
    format!("{{\n{}\n}}\n", lines.join(",\n"))
}

/// Writes the text through `<file>.tmp` and a rename, so that a reader never sees half of it. The error is the text
/// after `cannot write the summary file '<file>': `.
pub fn write_summary(file: &str, text: &str) -> Result<(), String> {
    let mut temporary = PathBuf::from(file).into_os_string();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    if let Err(error) = fs::write(&temporary, text) {
        let _ = fs::remove_file(&temporary);
        return Err(format!("cannot write '{}': {error}", temporary.display()));
    }
    fs::rename(&temporary, file).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        format!("cannot rename to '{file}': {error}")
    })
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;
    use crate::net_test::cli;
    use crate::net_test::histogram::Histogram;
    use crate::net_test::kit::summary_errors;
    use crate::net_test::statistics::LagStats;

    const START: i64 = 1_763_380_800_000_000_000;
    const MB: i64 = BYTES_PER_MB;

    fn options(args: &[&str]) -> Options {
        cli::parse(&args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>())
            .unwrap_or_else(|error| panic!("{args:?}: {error}"))
    }

    fn interval(end_epoch_ms: i64, warmup: bool, records: i64) -> IntervalRecord {
        IntervalRecord {
            start_epoch_nanos: (end_epoch_ms - 1000) * 1_000_000,
            end_epoch_nanos: end_epoch_ms * 1_000_000,
            warmup,
            records,
            rate: records,
            lag_mean_ns: None,
            lag_p50_ns: None,
            lag_p99_ns: None,
            lag_max_ns: None,
            publish_ns_per_event: Some(900),
            slip_max_ns: Some(30_000),
            slip_end_ns: Some(2_000),
            connections: 1,
        }
    }

    fn slip_interval(end_epoch_ms: i64, warmup: bool, max_ns: i64, end_ns: i64) -> IntervalRecord {
        IntervalRecord {
            slip_max_ns: Some(max_ns),
            slip_end_ns: Some(end_ns),
            ..interval(end_epoch_ms, warmup, 0)
        }
    }

    fn runtime_sample() -> RuntimeSample {
        RuntimeSample {
            heap_used_bytes: Some(50 * MB),
            heap_committed_bytes: Some(64 * MB),
            young_gc_count: Some(3),
            young_gc_time_ms: Some(30),
            full_gc_count: Some(1),
            full_gc_time_ms: Some(100),
            process_cpu_time_ns: Some(1_000_000_000),
        }
    }

    fn connectors() -> Vec<ConnectorSummary> {
        vec![
            ConnectorSummary {
                address: ":7001".to_owned(),
                v1: ConnectorV1 {
                    read_bytes: 10,
                    written_bytes: 20,
                    read_data_records: 0,
                    written_data_records: 100_000,
                    active_connections: 1,
                    closed_connections: 0,
                },
                v2: Some(ConnectorV2 {
                    read_sub_records: 1,
                    written_sub_records: 2,
                    data_read_lag_us: 3,
                    data_write_lag_us: 4,
                    read_rtt_us: 5,
                    write_rtt_us: 6,
                }),
            },
            ConnectorSummary {
                address: "127.0.0.1:7002".to_owned(),
                v1: ConnectorV1::default(),
                v2: None,
            },
        ]
    }

    fn producer_result() -> RunResult {
        let intervals = vec![
            interval(1_763_380_801_000, true, 400),
            interval(1_763_380_802_000, false, 600),
        ];
        let report = RunReport {
            start_epoch_nanos: START,
            end_epoch_nanos: START + 2_000_000_000,
            e2e_lag: None,
            publish: Some(PublishSummary {
                calls: 10,
                events: 1000,
                nanos: 900_000,
            }),
            slip: Some(slip_summary(&intervals, 2_000)),
            symbols_seen: None,
            runtime: runtime_summary(
                &TimedRuntime {
                    sample: runtime_sample(),
                    epoch_nanos: 1_000_000_000,
                },
                &TimedRuntime {
                    sample: RuntimeSample {
                        process_cpu_time_ns: Some(4_000_000_000),
                        ..runtime_sample()
                    },
                    epoch_nanos: 3_000_000_000,
                },
            ),
            intervals,
            connectors: connectors(),
        };
        RunResult {
            status: Status::Ok,
            error: None,
            report,
        }
    }

    fn consumer_result() -> RunResult {
        let mut histogram = Histogram::new();
        for lag in [0, 10, 1_000, 400_000] {
            histogram.record(lag);
        }
        let totals = E2eTotals {
            stats: LagStats {
                count: 4,
                negative: 1,
                sum: -4,
                max: 400_000,
            },
            histogram,
        };
        let intervals = vec![IntervalRecord {
            lag_mean_ns: Some(-12),
            lag_p50_ns: Some(0),
            lag_p99_ns: Some(381_183),
            lag_max_ns: Some(400_000),
            publish_ns_per_event: None,
            slip_max_ns: None,
            slip_end_ns: None,
            ..interval(1_763_380_810_000, false, 1000)
        }];
        RunResult {
            status: Status::StoppedBySignal,
            error: None,
            report: RunReport {
                start_epoch_nanos: START,
                end_epoch_nanos: START + 10_000_000_000,
                e2e_lag: Some(lag_summary(&totals)),
                symbols_seen: Some(100),
                intervals,
                ..RunReport::default()
            },
        }
    }

    fn parsed(text: &str) -> Value {
        serde_json::from_str(text).unwrap_or_else(|error| panic!("{error}: {text}"))
    }

    #[track_caller]
    fn assert_valid(text: &str) {
        let errors = summary_errors(text);
        assert!(errors.is_empty(), "{errors:?}\n{text}");
    }

    #[test]
    fn a_producer_summary_is_valid_against_the_schema_of_the_kit() {
        let args = [
            "p",
            ":7001",
            "-S",
            "100",
            "-r",
            "*/5k",
            "--warmup",
            "10s",
            "--summary",
            "summary.json",
        ];
        let options = options(&args);
        let text = format_summary(&options, Some("9.9.9"), &producer_result());
        assert_valid(&text);
        let value = parsed(&text);
        assert_eq!(value["side"], "producer");
        assert_eq!(value["tool"]["implementation"], "rust-api");
        assert_eq!(value["tool"]["name"], "NetTest");
        assert_eq!(value["tool"]["sdk_version"], "9.9.9");
        assert_eq!(value["tool"]["qd_version"], Value::Null);
        assert_eq!(
            value["tool"]["build"],
            serde_json::json!({"vendor": null, "march": null, "gc": null})
        );
        // The arguments are kept as they were given, the summary path included.
        assert_eq!(value["args"], serde_json::json!(args));
        assert_eq!(value["config"]["warmup_ms"], 10_000);
        assert_eq!(value["config"]["rate_limit"], 5_000);
        assert_eq!(value["config"]["reuse_effective"], false);
        assert_eq!(value["status"], "ok");
        assert_eq!(value["exit_code"], 0);
        assert_eq!(value["error"], Value::Null);
        assert_eq!(value["e2e_lag"], Value::Null);
        assert_eq!(value["symbols_seen"], Value::Null);
        assert_eq!(value["publish_call"]["ns_per_event_mean"], 900);
        assert_eq!(value["schedule_slip"]["max_ns"], 30_000);
        assert_eq!(value["schedule_slip"]["end_ns"], 2_000);
        assert_eq!(value["schedule_slip"]["growth_ns_per_s"], Value::Null);
        assert_eq!(value["intervals"][0]["phase"], "warmup");
        assert_eq!(value["intervals"][1]["phase"], "measure");
        assert_eq!(value["intervals"][1]["start_epoch_ms"], 1_763_380_801_000_i64);
        assert_eq!(value["time_resolution"], "ns");
        // The records of the intervals add up to the records of the run, and the rate is over its length.
        let records: i64 = value["intervals"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["records"].as_i64().unwrap())
            .sum();
        assert_eq!(records, 1000);
        assert_eq!(value["records"], records);
        assert_eq!(value["rate"], 500);
        assert_eq!(value["runtime"]["cpu_percent_mean"], 150);
        let connectors = value["connectors"].as_array().unwrap();
        assert_eq!(connectors.len(), 2);
        assert_eq!(connectors[0]["v1"]["written_data_records"], 100_000);
        assert_eq!(connectors[0]["v2"]["write_rtt_us"], 6);
        assert_eq!(connectors[1]["v2"], Value::Null);
    }

    #[test]
    fn a_consumer_summary_is_valid_against_the_schema_of_the_kit() {
        let options = options(&["c", "127.0.0.1:7001", "-S", "100", "-c", "stream"]);
        let text = format_summary(&options, None, &consumer_result());
        assert_valid(&text);
        let value = parsed(&text);
        assert_eq!(value["side"], "consumer");
        assert_eq!(value["status"], "stopped_by_signal");
        assert_eq!(value["exit_code"], 0);
        assert_eq!(value["tool"]["sdk_version"], Value::Null);
        assert_eq!(value["config"]["rate_limit"], Value::Null);
        assert_eq!(value["config"]["batch"], Value::Null);
        assert_eq!(value["config"]["reuse"], Value::Null);
        assert_eq!(value["config"]["reuse_effective"], Value::Null);
        assert_eq!(value["config"]["role"], "STREAM_FEED");
        assert_eq!(value["publish_call"], Value::Null);
        assert_eq!(value["schedule_slip"], Value::Null);
        assert_eq!(value["symbols_seen"], 100);
        assert_eq!(value["connectors"], serde_json::json!([]));
        let lag = &value["e2e_lag"];
        assert_eq!((lag["count"].as_i64(), lag["negative"].as_i64()), (Some(4), Some(1)));
        assert_eq!(lag["mean_ns"], -1);
        assert_eq!(lag["max_ns"], 400_000);
        assert_eq!(lag["p50_ns"], 10);
        assert_eq!(lag["histogram"]["highest"], 3_600_000_000_000_i64);
        assert_eq!(value["intervals"][0]["lag_mean_ns"], -12);
    }

    #[test]
    fn a_failed_run_carries_its_error_and_nothing_else() {
        let options = options(&["p", ":7001", "-S", "100"]);
        let message = "cannot connect \"x\"\n\ttab \\ done";
        let result = RunResult::failed(message.to_owned(), START, START + 5);
        let text = format_summary(&options, Some("1"), &result);
        assert_valid(&text);
        let value = parsed(&text);
        assert_eq!(value["status"], "error");
        assert_eq!(value["exit_code"], 1);
        assert_eq!(value["error"], message);
        assert_eq!(value["records"], 0);
        assert_eq!(value["rate"], 0);
        assert_eq!(value["intervals"], serde_json::json!([]));
        assert_eq!(value["publish_call"], Value::Null);
        assert_eq!(value["schedule_slip"], Value::Null);
        assert_eq!(value["runtime"]["heap_used_mb"], Value::Null);
        // The empty arrays are written as [] and the file ends with a new line.
        assert!(text.contains("  \"intervals\": [],\n"), "{text}");
        assert!(text.ends_with("  \"connectors\": []\n}\n"), "{text}");
    }

    #[test]
    fn every_status_and_the_time_resolution_are_valid() {
        let options = options(&["c", "127.0.0.1:7001", "-S", "1", "-Ddxscheme.nanoTime=false"]);
        for status in [
            Status::Ok,
            Status::StoppedBySignal,
            Status::NoData,
            Status::NanoMissing,
            Status::Error,
        ] {
            let result = RunResult {
                status,
                error: (status != Status::Ok).then(|| "x".to_owned()),
                ..consumer_result()
            };
            let text = format_summary(&options, None, &result);
            assert_valid(&text);
            let value = parsed(&text);
            assert_eq!(value["status"], status.name());
            assert_eq!(value["exit_code"], i32::from(status.exit_code()));
            assert_eq!(value["time_resolution"], "ms");
            assert_eq!(value["config"]["nano_time"], false);
        }
    }

    #[test]
    fn the_checker_is_not_vacuous_for_the_summary() {
        let mut unnamed = options(&["p", ":7001", "-S", "100"]);
        unnamed.name.clear();
        let errors = summary_errors(&format_summary(&unnamed, None, &producer_result()));
        assert!(
            errors
                .iter()
                .any(|error| error.contains(".name: shorter than the minLength")),
            "{errors:?}"
        );
        // A status outside the enumeration and a missing key are found too.
        let text = format_summary(&options(&["p", ":7001", "-S", "100"]), None, &producer_result());
        let errors = summary_errors(&text.replace("\"status\": \"ok\"", "\"status\": \"fine\""));
        assert!(
            errors.iter().any(|error| error.contains(".status: is not one of")),
            "{errors:?}"
        );
        let errors = summary_errors(&text.replace("  \"records\": 1000,\n", ""));
        assert!(
            errors.iter().any(|error| error.contains(".records: is required")),
            "{errors:?}"
        );
    }

    #[test]
    fn the_top_level_keys_come_in_the_order_of_the_c_tool() {
        let text = format_summary(&options(&["p", ":7001", "-S", "100"]), None, &producer_result());
        let keys: Vec<&str> = text
            .lines()
            .filter_map(|line| line.strip_prefix("  \""))
            .filter_map(|line| line.split_once('"').map(|(key, _)| key))
            .collect();
        let expected = [
            "schema",
            "spec_version",
            "tool",
            "side",
            "name",
            "args",
            "config",
            "start_epoch_ns",
            "end_epoch_ns",
            "status",
            "exit_code",
            "error",
            "records",
            "rate",
            "time_source",
            "time_resolution",
            "intervals",
            "e2e_lag",
            "publish_call",
            "schedule_slip",
            "symbols_seen",
            "runtime",
            "connectors",
        ];
        assert_eq!(keys, expected);
        // An object is on one line with `, `; the elements of an array have four spaces.
        assert!(
            text.contains("\n  \"intervals\": [\n    {\"start_epoch_ms\": 1763380800000, "),
            "{text}"
        );
        assert!(text.contains("}\n  ],\n  \"e2e_lag\": null,\n"), "{text}");
    }

    #[test]
    fn strings_are_escaped() {
        assert_eq!(json_string("a\"b\\c"), "\"a\\\"b\\\\c\"");
        assert_eq!(json_string("\n\r\t\u{1}\u{1f}"), "\"\\n\\r\\t\\u0001\\u001f\"");
        // Other characters, the DEL and the non-ASCII ones, stay as they are.
        assert_eq!(json_string("é\u{7f}/"), "\"é\u{7f}/\"");
        // A name and an argument with the special characters come back unchanged.
        let options = options(&["p", ":7001", "-S", "1", "-n", "a\"b\\c\td"]);
        let value = parsed(&format_summary(&options, None, &producer_result()));
        assert_eq!(value["name"], "a\"b\\c\td");
        assert_eq!(value["args"][5], "a\"b\\c\td");
    }

    #[test]
    fn the_slip_is_the_largest_of_the_measured_intervals_and_grows_per_second_of_their_ends() {
        // A warm-up interval with a larger slip than any measured one, then three measured intervals over 2 s.
        let intervals = [
            slip_interval(1000, true, 90_000_000, 80_000_000),
            slip_interval(2000, false, 5_000_000, 1_000_000),
            slip_interval(3000, false, 7_000_000, 4_000_000),
            slip_interval(4000, false, 6_000_000, 9_000_000),
        ];
        let slip = slip_summary(&intervals, 9_500_000);
        assert_eq!(slip.max_ns, 7_000_000);
        assert_eq!(slip.end_ns, 9_500_000);
        // 8 ms of growth over the 2 s between the ends of the first and the last measured interval.
        assert_eq!(slip.growth_ns_per_s, Some(4_000_000));
        // A slip that shrinks grows negatively.
        let shrinking = slip_summary(&[slip_interval(1000, false, 8, 8), slip_interval(1500, false, 8, 2)], 2);
        assert_eq!(shrinking.growth_ns_per_s, Some(-12));
    }

    #[test]
    fn without_measured_intervals_the_slip_is_the_largest_of_all_and_one_measured_has_no_growth() {
        let warmup_only = slip_summary(
            &[
                slip_interval(1000, true, 3_000, 1_000),
                slip_interval(2000, true, 8_000, 2_000),
            ],
            2_000,
        );
        assert_eq!(warmup_only.max_ns, 8_000);
        assert_eq!(warmup_only.growth_ns_per_s, None);
        let one_measured = slip_summary(
            &[
                slip_interval(1000, true, 3_000, 1_000),
                slip_interval(2000, false, 2_000, 2_000),
            ],
            2_000,
        );
        assert_eq!(one_measured.max_ns, 2_000);
        assert_eq!(one_measured.growth_ns_per_s, None);
        assert_eq!(slip_summary(&[], 0), SlipSummary::default());
        // Two measured intervals that end at the same millisecond have no growth.
        let flat = slip_summary(&[slip_interval(1000, false, 1, 1), slip_interval(1000, false, 2, 5)], 5);
        assert_eq!(flat.growth_ns_per_s, None);
    }

    #[test]
    fn the_runtime_is_the_heap_at_the_end_and_the_growth_between_the_samples() {
        let first = TimedRuntime {
            sample: RuntimeSample {
                heap_used_bytes: Some(10 * MB),
                heap_committed_bytes: Some(20 * MB),
                ..runtime_sample()
            },
            epoch_nanos: 1_000_000_000,
        };
        let last_sample = RuntimeSample {
            heap_used_bytes: Some(50 * MB + MB / 2),
            heap_committed_bytes: Some(64 * MB),
            young_gc_count: Some(10),
            young_gc_time_ms: Some(75),
            full_gc_count: Some(2),
            full_gc_time_ms: Some(160),
            process_cpu_time_ns: Some(4_000_000_000),
        };
        let last = TimedRuntime {
            sample: last_sample,
            epoch_nanos: 3_000_000_000,
        };
        // 3 s of CPU time over 2 s of the run: 150 %.
        let runtime = runtime_summary(&first, &last);
        assert_eq!(runtime.heap_used_mb, Some(50));
        assert_eq!(runtime.heap_committed_mb, Some(64));
        assert_eq!(runtime.gc_count, Some(8));
        assert_eq!(runtime.gc_time_ms, Some(105));
        assert_eq!(runtime.cpu_percent_mean, Some(150));
        // An absent value, a decreasing count and an empty period leave the values absent.
        let partial = TimedRuntime {
            sample: RuntimeSample {
                heap_used_bytes: None,
                full_gc_count: Some(0),
                young_gc_time_ms: None,
                ..last_sample
            },
            epoch_nanos: 1_000_000_000,
        };
        let runtime = runtime_summary(&first, &partial);
        assert_eq!(runtime.heap_used_mb, None);
        assert_eq!(runtime.heap_committed_mb, Some(64));
        assert_eq!(runtime.gc_count, None);
        assert_eq!(runtime.gc_time_ms, None);
        assert_eq!(runtime.cpu_percent_mean, None);
        // A run without the samples reports nothing.
        let report = common_report(0, 1, ReporterOutcome::default());
        assert_eq!(report.runtime, RuntimeSummary::default());
    }

    #[test]
    fn the_lag_totals_have_the_percentiles_of_the_histogram_and_nulls_without_lags() {
        let empty = lag_summary(&E2eTotals::default());
        assert_eq!(empty, LagSummary::default());
        let options = options(&["c", "127.0.0.1:7001", "-S", "1"]);
        let result = RunResult {
            report: RunReport {
                e2e_lag: Some(empty),
                symbols_seen: Some(0),
                ..RunReport::default()
            },
            ..consumer_result()
        };
        let text = format_summary(&options, None, &result);
        assert_valid(&text);
        let lag = &parsed(&text)["e2e_lag"];
        assert_eq!(lag["count"], 0);
        assert_eq!(lag["p50_ns"], Value::Null);
        assert_eq!(lag["mean_ns"], Value::Null);
        // The mean is divided toward zero: -4 over 4 lags.
        let negative = E2eTotals {
            stats: LagStats {
                count: 2,
                negative: 2,
                sum: -3,
                max: -1,
            },
            histogram: {
                let mut histogram = Histogram::new();
                histogram.record(-1);
                histogram.record(-2);
                histogram
            },
        };
        let lag = lag_summary(&negative);
        assert_eq!((lag.mean_ns, lag.max_ns, lag.p50_ns), (Some(-1), Some(-1), Some(0)));
    }

    #[test]
    fn the_producer_report_keeps_the_slip_only_with_a_schedule() {
        let counters = PublishCounters::default();
        counters.calls.store(4, std::sync::atomic::Ordering::Relaxed);
        counters.nanos.store(1000, std::sync::atomic::Ordering::Relaxed);
        counters.slip_last_nanos.store(7, std::sync::atomic::Ordering::Relaxed);
        let reporter = || ReporterOutcome {
            intervals: vec![interval(2000, false, 5), interval(3000, false, 6)],
            ..ReporterOutcome::default()
        };
        let totals = |scheduled| ProducerTotals {
            counters: &counters,
            events: 11,
            scheduled,
        };
        let flat = producer_report(1, 2, reporter(), totals(false), Vec::new());
        assert_eq!(flat.slip, None);
        assert_eq!(
            flat.publish,
            Some(PublishSummary {
                calls: 4,
                events: 11,
                nanos: 1000
            })
        );
        let scheduled = producer_report(1, 2, reporter(), totals(true), Vec::new());
        assert_eq!(scheduled.slip.map(|slip| slip.end_ns), Some(7));
    }

    #[test]
    fn the_file_is_written_through_a_temporary_file_and_replaces_the_old_one() {
        let directory = TempDirectory::new("write");
        let file = directory.path("summary.json");
        write_summary(&file, "{}\n").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "{}\n");
        assert!(!std::path::Path::new(&format!("{file}.tmp")).exists());
        write_summary(&file, "{\"a\":1}\n").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "{\"a\":1}\n");
    }

    #[test]
    fn a_file_that_cannot_be_written_is_an_error() {
        let directory = TempDirectory::new("fail");
        let error = write_summary(&directory.path("no-such-dir/summary.json"), "{}\n").unwrap_err();
        assert!(error.contains("no-such-dir"), "{error}");
        // The temporary file is a directory: the write fails and leaves that directory alone.
        let file = directory.path("blocked.json");
        std::fs::create_dir(format!("{file}.tmp")).unwrap();
        assert!(write_summary(&file, "{}\n").is_err());
        assert!(std::path::Path::new(&format!("{file}.tmp")).is_dir());
        // The target is a directory: the rename fails and the temporary file goes.
        let target = directory.path("target.json");
        std::fs::create_dir(&target).unwrap();
        assert!(write_summary(&target, "{}\n").is_err());
        assert!(!std::path::Path::new(&format!("{target}.tmp")).exists());
    }

    /// A directory in the temporary directory, removed when the test ends.
    struct TempDirectory(std::path::PathBuf);

    impl TempDirectory {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("dxfeed-graal-tools-summary-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            TempDirectory(path)
        }

        fn path(&self, name: &str) -> String {
            self.0.join(name).to_string_lossy().into_owned()
        }
    }

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
