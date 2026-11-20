//! The NetTest tool: publishes generated Trade or TimeAndSale events and prints the statistics of the QD tool
//! `NetTest p`, as the NetTest publisher of the C API does.
//!
//! It parses its own command line and runs before the common system properties of the tools are set, so that its
//! `-X` and `-XX:` options reach the isolate.

mod cli;
mod events;
mod generator;
mod histogram;
#[cfg(test)]
mod kit;
mod limiter;
mod statistics;
mod symbols;

use std::panic::{self, AssertUnwindSafe};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use dxfeed_graal::{DXEndpoint, DXPublisher, Error, MessageConnectorKind, MessageConnectorState, system};

use crate::console;
use crate::help::Info;

use self::cli::{Mode, OPTION_LINES, Options, SYNOPSIS};
use self::events::EventPool;
use self::generator::{Counters, PublishCounters, Settings, run_generator};
use self::limiter::{ScheduleLimiter, Stop};
use self::statistics::{
    InstanceTotals, IntervalTaken, ReporterSettings, RuntimeSample, Side, StatisticsSample, run_reporter, system_clock,
};
use self::symbols::Symbols;

pub const INFO: Info = Info {
    name: "NetTest",
    short_description: "Publishes generated events and prints the statistics of QD NetTest.",
    description: "\nPublishes generated Trade or TimeAndSale events through publisher endpoints at an optional total \
                  rate and prints the statistics lines of the QD tool NetTest p, as the NetTest of the C API does.\n",
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
    if options.mode == Mode::Consumer {
        return fail("the mode c is not implemented yet", EXIT_FAILURE);
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
    // The system properties come before the first endpoint.
    for (key, value) in &options.properties {
        if system::set_property(key, value).is_err() {
            return fail(&format!("cannot set the system property '{key}'"), EXIT_FAILURE);
        }
    }
    let pool = symbols::build_symbol_pool(&options);
    if let Some(warning) = pool.as_ref().ok().and_then(|pool| pool.warning.as_ref()) {
        eprintln!("NetTest: warning: {warning}");
    }
    let pool = match pool {
        Ok(pool) => pool,
        Err(error) => return fail(&error, EXIT_FAILURE),
    };
    // Ctrl+C and SIGTERM end the run like --duration does: the last statistics are printed and the endpoints close.
    let stop = Arc::new(Stop::default());
    let handler_stop = Arc::clone(&stop);
    if let Err(error) = ctrlc::set_handler(move || handler_stop.request()) {
        return fail(&format!("cannot handle Ctrl+C: {error}"), EXIT_FAILURE);
    }
    match run(&options, &pool.symbols, &stop) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => fail(&error, EXIT_FAILURE),
    }
}

/// One publisher endpoint with its counters.
struct Instance {
    name: String,
    endpoint: DXEndpoint,
    publisher: DXPublisher,
    counters: Counters,
}

impl Instance {
    /// Creates the endpoint of the name and its publisher.
    fn create(name: String, options: &Options) -> Result<Self, String> {
        let endpoint = DXEndpoint::new_builder()
            .with_role(options.role())
            .with_name(&name)
            .with_property(DXEndpoint::DXSCHEME_NANO_TIME_PROPERTY, options.nano_time())
            .build()
            .map_err(|error| format!("cannot create the endpoint {name}: {error}"))?;
        let publisher = endpoint.get_publisher();
        Ok(Instance {
            name,
            endpoint,
            publisher,
            counters: Counters::default(),
        })
    }

    /// Connects the endpoint. A server socket must bind: the connector of a bound one is connected within
    /// milliseconds, one that cannot bind stays connecting, so the start waits until every server socket connector is
    /// connected. A client address never waits: with no peer its connector stays connecting, a normal state.
    fn connect(&self, address: &str) -> Result<(), String> {
        let name = &self.name;
        self.endpoint
            .connect(address)
            .map_err(|error| format!("cannot connect {name} to '{address}': {error}"))?;
        let deadline = Instant::now() + SERVER_START_TIMEOUT;
        loop {
            let pending = self
                .endpoint
                .get_connectors()
                .unwrap_or_default()
                .iter()
                .any(|connector| {
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

    /// The open connections of all connectors of the endpoint.
    fn connections(&self) -> i64 {
        self.endpoint
            .get_connectors()
            .unwrap_or_default()
            .iter()
            .filter_map(|connector| connector.get_connection_count().ok())
            .map(i64::from)
            .sum()
    }

    fn totals(&self) -> InstanceTotals {
        InstanceTotals {
            records: self.counters.records.load(Ordering::Relaxed),
            weighted_nanos: self.counters.weighted_nanos.load(Ordering::Relaxed),
        }
    }
}

fn close(instances: &[Instance]) {
    for instance in instances {
        let _ = instance.endpoint.close_and_await_termination();
    }
}

/// Starts the instances, runs the generators and the reporter until the stop, the duration or a failed publish, then
/// joins the threads (the generators first, then the reporter with the last statistics) and closes the endpoints.
fn run(options: &Options, symbols: &Symbols, stop: &Stop) -> Result<(), String> {
    let mut instances = Vec::with_capacity(options.instances);
    let mut pools = Vec::with_capacity(options.instances);
    for index in 1..=options.instances {
        let started = Instance::create(format!("{}.{index}", options.name), options).and_then(|instance| {
            let connected = instance.connect(&options.address);
            // The endpoint that failed to start closes with the others.
            instances.push(instance);
            connected
        });
        if let Err(error) = started {
            close(&instances);
            return Err(error);
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
    let publish = PublishCounters::default();
    let failure = Mutex::new(None);
    let reporter_stop = Stop::default();
    std::thread::scope(|scope| {
        let instances = &instances;
        let reporter = scope.spawn(|| {
            // The interval records are what the summary will keep.
            let _outcome = run_reporter(
                ReporterSettings {
                    name: &options.name,
                    side: Side::Producer,
                    period_millis: options.period_millis,
                    warmup_millis: options.warmup_millis,
                },
                system_clock(&reporter_stop),
                || StatisticsSample {
                    instances: instances.iter().map(Instance::totals).collect(),
                    connections: instances.iter().map(Instance::connections).sum(),
                    // A failed read prints n/a values, as in the C tool.
                    runtime: Some(
                        system::get_runtime_statistics()
                            .map(|statistics| RuntimeSample::from_statistics(&statistics))
                            .unwrap_or_default(),
                    ),
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
                        lags: None,
                    }
                },
                console::print,
            );
        });
        let generators: Vec<_> = instances
            .iter()
            .zip(pools)
            .map(|(instance, mut pool)| {
                let (limiter, failure, publish) = (limiter.as_ref(), &failure, &publish);
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
                    failure
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .get_or_insert(error);
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
        if reporter.join().is_err() {
            failure
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get_or_insert_with(|| "the reporter thread panicked".to_owned());
        }
    });
    close(&instances);
    match failure.into_inner().unwrap_or_else(|poisoned| poisoned.into_inner()) {
        Some(error) => Err(error),
        None => Ok(()),
    }
}
