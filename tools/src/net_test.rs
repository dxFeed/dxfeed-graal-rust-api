//! The NetTest tool: publishes generated Trade or TimeAndSale events (`p`) or receives them and reports the
//! end-to-end lag (`c`), printing the statistics of the QD tool NetTest, as the NetTest of the C API does.
//!
//! It parses its own command line and runs before the common system properties of the tools are set, so that its
//! `-X` and `-XX:` options reach the isolate.

mod cli;
mod consumer;
mod events;
mod exposition;
mod generator;
mod histogram;
mod http;
#[cfg(test)]
mod kit;
mod limiter;
mod signals;
mod statistics;
mod summary;
mod symbols;

use std::panic::{self, AssertUnwindSafe};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use dxfeed_graal::{DXEndpoint, DXPublisher, Error, MessageConnectorKind, MessageConnectorState, system};

use crate::console;
use crate::help::Info;

use self::cli::{Mode, OPTION_LINES, Options, SYNOPSIS};
use self::consumer::{WATCHDOG_VARIABLE, WatchdogTimeouts, run_consumer};
use self::events::{EventPool, now_epoch_nanos};
use self::exposition::LiveMetrics;
use self::generator::{Counters, Settings, run_generator};
use self::http::MetricsServer;
use self::limiter::{ScheduleLimiter, Stop, lock_ignoring_poison};
use self::signals::SignalWatcher;
use self::statistics::{
    IntervalTaken, ReporterOutcome, ReporterSettings, RuntimeSample, Side, StatisticsSample, run_reporter, system_clock,
};
use self::summary::{
    ProducerTotals, RunReport, RunResult, ToolInfo, connector_summaries, connector_totals, format_summary,
    producer_report, write_summary,
};
use self::symbols::Symbols;

pub const INFO: Info = Info {
    name: "NetTest",
    short_description: "Publishes (p) or receives (c) generated events and prints QD NetTest statistics.",
    description: "\nMode p publishes generated Trade or TimeAndSale events through publisher endpoints at an optional \
                  total rate; mode c receives them through feed endpoints and reports the end-to-end lag. Both print \
                  the statistics lines of the QD tool NetTest, as the NetTest of the C API does, and can write a JSON \
                  summary of the run (--summary) and serve Prometheus metrics (--metrics-port).\n",
    usage: &[SYNOPSIS],
    additional_info: OPTION_LINES,
    args: &[],
};

const EXIT_FAILURE: u8 = 1;
const EXIT_BAD_ARGUMENTS: u8 = 2;
/// The code of the isolate creation when the isolate rejects an argument (`ARGUMENT_PARSING_FAILED` of GraalVM).
const ISOLATE_ARGUMENT_PARSING_FAILED: i32 = 22;
/// How long the start waits for a server socket to bind: the connector of a bound one reports itself as connected
/// within milliseconds; the connector of one that could not bind (the port is taken) stays connecting.
const SERVER_START_TIMEOUT: Duration = Duration::from_millis(3000);

/// How a run ended: the `status` of the summary file and, through it, the exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// The duration ended the run.
    Ok,
    /// SIGINT or SIGTERM ended the run.
    StoppedBySignal,
    /// The watchdog of the consumer found open connections without records.
    NoData,
    /// The first events of the consumer all lack the nanosecond part.
    NanoMissing,
    /// A failure at run time: an endpoint, a port in use, a publishing call.
    Error,
}

impl Status {
    /// The name of the status in the summary file.
    pub fn name(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::StoppedBySignal => "stopped_by_signal",
            Status::NoData => "no_data",
            Status::NanoMissing => "nano_missing",
            Status::Error => "error",
        }
    }

    pub fn exit_code(self) -> u8 {
        match self {
            Status::Ok | Status::StoppedBySignal => 0,
            Status::NoData | Status::NanoMissing | Status::Error => EXIT_FAILURE,
        }
    }
}

fn fail(message: &str, code: u8) -> ExitCode {
    eprintln!("NetTest: {message}");
    ExitCode::from(code)
}

/// Runs `Tools NetTest <args>`, the arguments after the tool name; returns the exit code.
pub fn main(args: &[String]) -> ExitCode {
    if args.is_empty() {
        console::print(&cli::usage_text());
        return ExitCode::from(EXIT_BAD_ARGUMENTS);
    }
    let options = match cli::parse(args) {
        Ok(options) => options,
        Err(error) => {
            eprint!("NetTest: {error}\n\n{}", cli::usage_text());
            return ExitCode::from(EXIT_BAD_ARGUMENTS);
        }
    };
    if options.help {
        console::print(&cli::usage_text());
        return ExitCode::SUCCESS;
    }
    if let Err(error) = system::set_isolate_arguments(&options.isolate_arguments) {
        return fail(&error.to_string(), EXIT_BAD_ARGUMENTS);
    }
    // The isolate is created lazily: the first call creates it here, so that a rejected option is a bad command line.
    match system::get_runtime_statistics() {
        Err(Error::Graal(ISOLATE_ARGUMENT_PARSING_FAILED)) => {
            // GraalVM prints the reason above; the line names what the tool passed, as every NetTest does.
            let passed = options.isolate_arguments.join(" ");
            return fail(
                &format!("the isolate rejected one of the options: {passed}"),
                EXIT_BAD_ARGUMENTS,
            );
        }
        Err(Error::Graal(code)) => return fail(&format!("cannot create the isolate (error {code})"), EXIT_FAILURE),
        _ => {}
    }
    // SIGINT and SIGTERM end the run like --duration does: the last statistics are printed and the endpoints close.
    // The watcher comes once the isolate exists, so that a signal while the symbols are read also ends with the
    // summary; a second signal ends the process.
    let stop = Arc::new(Stop::default());
    let signalled = Arc::new(AtomicBool::new(false));
    let signal_watcher = SignalWatcher::install(Arc::clone(&stop), Arc::clone(&signalled));
    // From here on every end writes the summary: a failure before the run is the status `error` with nothing else.
    let before_run = now_epoch_nanos();
    let failed_before_run = |message: String| {
        eprintln!("NetTest: {message}");
        finish(&options, &RunResult::failed(message, before_run, now_epoch_nanos()))
    };
    // The system properties come before the first endpoint.
    for (key, value) in &options.properties {
        if system::set_property(key, value).is_err() {
            return failed_before_run(format!("cannot set the system property '{key}'"));
        }
    }
    // The metrics port is bound before the first endpoint, so that a port in use fails the start and a scraper finds
    // the tool as soon as its line is printed.
    let live = Arc::new(LiveMetrics::for_run(&options, system::get_version().ok().as_deref()));
    let server = match options.metrics_port {
        Some(port) => {
            let scraped = Arc::clone(&live);
            match MetricsServer::start(&options.metrics_host, port, move || scraped.render()) {
                Ok(server) => {
                    console::println(&format!(
                        "{{*NetTestMetrics-{}*}} http://{}:{}/metrics",
                        options.name,
                        options.metrics_host,
                        server.port()
                    ));
                    Some(server)
                }
                Err(error) => return failed_before_run(error),
            }
        }
        None => None,
    };
    let pool = match symbols::build_symbol_pool(&options) {
        Ok(pool) => {
            if let Some(warning) = &pool.warning {
                eprintln!("NetTest: warning: {warning}");
            }
            pool
        }
        Err(error) => return failed_before_run(error),
    };
    // A signal before the endpoints start: the summary of the empty run, without the endpoints.
    if signal_watcher.received() {
        drop(server);
        let result = RunResult {
            status: Status::StoppedBySignal,
            error: None,
            report: RunReport::at(before_run, now_epoch_nanos()),
        };
        return finish(&options, &result);
    }
    // The run starts here, after the symbols and before the endpoints.
    let started = now_epoch_nanos();
    let result = if options.mode == Mode::Consumer {
        let timeouts = WatchdogTimeouts::from_variable(std::env::var(WATCHDOG_VARIABLE).ok().as_deref());
        run_consumer(&options, &pool.symbols, &stop, &signalled, timeouts, &live).into_result(started)
    } else {
        run(&options, &pool.symbols, &stop, &signalled, started, &live)
    };
    // The server stops with the run; the summary does not wait for a scraper.
    drop(server);
    if let Some(error) = &result.error {
        eprintln!("NetTest: {error}");
    }
    finish(&options, &result)
}

/// Writes the summary file when `--summary` is given and returns the exit code of the status; a summary that cannot be
/// written makes it 1 when it would be 0.
fn finish(options: &Options, result: &RunResult) -> ExitCode {
    let code = result.status.exit_code();
    let Some(file) = &options.summary_file else {
        return ExitCode::from(code);
    };
    let text = format_summary(options, &ToolInfo::read(), result);
    match write_summary(file, &text) {
        Ok(()) => ExitCode::from(code),
        Err(error) => {
            eprintln!("NetTest: cannot write the summary file '{file}': {error}");
            ExitCode::from(if code == 0 { EXIT_FAILURE } else { code })
        }
    }
}

/// One publisher endpoint with its counters.
struct Instance {
    name: String,
    endpoint: DXEndpoint,
    publisher: DXPublisher,
    counters: Arc<Counters>,
}

impl Instance {
    /// Creates the endpoint of the name and its publisher.
    fn create(name: String, options: &Options, counters: Arc<Counters>) -> Result<Self, String> {
        let endpoint = build_endpoint(&name, options)?;
        let publisher = endpoint.get_publisher();
        Ok(Instance {
            name,
            endpoint,
            publisher,
            counters,
        })
    }

    fn connect(&self, address: &str) -> Result<(), String> {
        connect_endpoint(&self.endpoint, &self.name, address)
    }
}

/// The runtime values of the isolate; a failed read gives a sample of absent values, which print as n/a.
fn read_runtime() -> RuntimeSample {
    system::get_runtime_statistics()
        .map(|statistics| RuntimeSample::from_statistics(&statistics))
        .unwrap_or_default()
}

/// Creates the endpoint of the role with `dxscheme.nanoTime` on its builder.
fn build_endpoint(name: &str, options: &Options) -> Result<DXEndpoint, String> {
    DXEndpoint::new_builder()
        .with_role(options.role())
        .with_name(name)
        .with_property(DXEndpoint::DXSCHEME_NANO_TIME_PROPERTY, options.nano_time())
        .build()
        .map_err(|error| format!("cannot create the endpoint {name}: {error}"))
}

/// Connects the endpoint to the address. A server socket must bind: the connector of a bound one is connected within
/// milliseconds, one that cannot bind stays connecting, so the start waits until every server socket connector is
/// connected. A client address never waits: with no peer its connector stays connecting, a normal state.
fn connect_endpoint(endpoint: &DXEndpoint, name: &str, address: &str) -> Result<(), String> {
    endpoint
        .connect(address)
        .map_err(|error| format!("cannot connect {name} to '{address}': {error}"))?;
    let deadline = Instant::now() + SERVER_START_TIMEOUT;
    loop {
        let pending = endpoint.get_connectors().unwrap_or_default().iter().any(|connector| {
            connector.get_kind().ok() == Some(MessageConnectorKind::ServerSocket)
                && connector
                    .get_state()
                    .is_ok_and(|state| state != MessageConnectorState::Connected)
        });
        if !pending {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "the server socket '{address}' ({name}) did not start in {} ms (is the port already in use?)",
                SERVER_START_TIMEOUT.as_millis()
            ));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The sample of one reporter tick over the instances (their endpoints and counters): the open connections, the
/// runtime values and the totals. With `serving` it also stores what the metrics read; without a server it makes no
/// connector calls, which would be calls into the isolate for nobody.
fn reporter_sample<'a>(
    live: &LiveMetrics,
    serving: bool,
    instances: impl Iterator<Item = (&'a DXEndpoint, &'a Counters)> + Clone,
) -> StatisticsSample {
    let connections: Vec<i64> = instances
        .clone()
        .map(|(endpoint, _)| connections_of(endpoint))
        .collect();
    let runtime = read_runtime();
    if serving {
        live.store(
            connections.clone(),
            connector_totals(instances.clone().map(|(endpoint, _)| endpoint)),
            runtime,
        );
    }
    StatisticsSample {
        instances: instances.map(|(_, counters)| counters.totals()).collect(),
        connections: connections.iter().sum(),
        runtime: Some(runtime),
    }
}

/// The open connections of all connectors of the endpoint.
fn connections_of(endpoint: &DXEndpoint) -> i64 {
    endpoint
        .get_connectors()
        .unwrap_or_default()
        .iter()
        .filter_map(|connector| connector.get_connection_count().ok())
        .map(i64::from)
        .sum()
}

fn close(instances: &[Instance]) {
    for instance in instances {
        let _ = instance.endpoint.close_and_await_termination();
    }
}

/// Starts the instances, runs the generators and the reporter until the stop, the duration or a failed publish, then
/// joins the threads (the generators first, then the reporter with the last statistics) and closes the endpoints.
fn run(
    options: &Options,
    symbols: &Symbols,
    stop: &Stop,
    signalled: &AtomicBool,
    started: i64,
    live: &LiveMetrics,
) -> RunResult {
    let mut instances = Vec::with_capacity(options.instances);
    let mut pools = Vec::with_capacity(options.instances);
    for index in 1..=options.instances {
        let counters = Arc::clone(&live.records[index - 1]);
        let connected = Instance::create(format!("{}.{index}", options.name), options, counters).and_then(|instance| {
            let connected = instance.connect(&options.address);
            // The endpoint that failed to start closes with the others.
            instances.push(instance);
            connected
        });
        if let Err(error) = connected {
            close(&instances);
            return RunResult::failed(error, started, now_epoch_nanos());
        }
        pools.push(EventPool::new(options.event, symbols, index as u64));
    }
    let settings = Settings {
        batch: options.batch,
        reuse: options.reuse,
    };
    let limiter = options
        .records_per_second
        .map(|rate| ScheduleLimiter::new(rate, options.batch as i64, stop));
    let publish = &live.publish;
    // The metrics read what the reporter stores, so without a server it stores nothing.
    let serving = options.metrics_port.is_some();
    let failure = Mutex::new(None);
    let reporter_stop = Stop::default();
    let reporter = std::thread::scope(|scope| {
        let instances = &instances;
        let reporter = scope.spawn(|| {
            run_reporter(
                ReporterSettings {
                    name: &options.name,
                    side: Side::Producer,
                    period_millis: options.period_millis,
                    warmup_millis: options.warmup_millis,
                },
                system_clock(&reporter_stop),
                || {
                    let sampled = instances
                        .iter()
                        .map(|instance| (&instance.endpoint, &*instance.counters));
                    reporter_sample(live, serving, sampled)
                },
                || {
                    // Taken once per interval, with or without a schedule; only a schedule reports it.
                    let slip_max = publish.take_interval_slip_max_nanos();
                    let slip_end = publish.slip_last_nanos.load(Ordering::Relaxed);
                    let scheduled = limiter.is_some();
                    IntervalTaken {
                        warmup_start_epoch_nanos: publish.first_batch_epoch_nanos(),
                        publish_nanos_total: publish.nanos.load(Ordering::Relaxed),
                        slip_max_nanos: scheduled.then_some(slip_max),
                        slip_end_nanos: scheduled.then_some(slip_end),
                        ..IntervalTaken::default()
                    }
                },
                console::print,
            )
        });
        let generators: Vec<_> = instances
            .iter()
            .zip(pools)
            .map(|(instance, mut pool)| {
                let (limiter, failure) = (limiter.as_ref(), &failure);
                scope.spawn(move || {
                    let published = panic::catch_unwind(AssertUnwindSafe(|| {
                        run_generator(
                            &mut pool,
                            settings,
                            limiter,
                            stop,
                            &instance.counters,
                            publish,
                            |events| instance.publisher.publish_events(events),
                        )
                    }));
                    let error = match published {
                        Ok(Ok(())) => return,
                        Ok(Err(error)) => format!("publishing failed: {error}"),
                        Err(_) => "a generator thread panicked".to_owned(),
                    };
                    lock_ignoring_poison(failure).get_or_insert(error);
                    stop.request();
                })
            })
            .collect();
        match options.duration_millis {
            Some(duration) => {
                stop.wait_for(Duration::from_millis(duration as u64));
            }
            None => stop.wait(),
        }
        stop.request();
        for generator in generators {
            let _ = generator.join();
        }
        reporter_stop.request();
        reporter.join().unwrap_or_else(|_| {
            lock_ignoring_poison(&failure).get_or_insert_with(|| "the reporter thread panicked".to_owned());
            ReporterOutcome::default()
        })
    });
    // The report is taken before the endpoints close: a closed connector reads 0 for its records.
    let ended = now_epoch_nanos();
    let connectors = connector_summaries(instances.iter().map(|instance| &instance.endpoint));
    close(&instances);
    let error = failure.into_inner().unwrap_or_else(|poisoned| poisoned.into_inner());
    let status = match (&error, signalled.load(Ordering::SeqCst)) {
        (Some(_), _) => Status::Error,
        (None, true) => Status::StoppedBySignal,
        (None, false) => Status::Ok,
    };
    let totals = ProducerTotals {
        counters: publish,
        events: instances
            .iter()
            .map(|instance| instance.counters.totals().records)
            .sum(),
        scheduled: options.records_per_second.is_some(),
    };
    RunResult {
        status,
        error,
        report: producer_report(started, ended, reporter, totals, connectors),
    }
}
