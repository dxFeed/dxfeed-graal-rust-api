//! The command line of NetTest, the specification `nettest/1`: the options, their parsing and validation, the
//! effective role and the usage text.
//!
//! The parser follows the order of the C tool: the first error wins. Every argument is taken left to right: `-D` and
//! `-X` by their prefix, the address, then the lookup of the option by its exact name, the check of the mode, the value
//! (after `=` or the next argument) and the option itself. The rules between the options come after the last argument.

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};

use dxfeed_graal::Role;

/// The version of the specification that the tool follows; the usage ends with it.
pub const SPEC_VERSION: &str = "nettest/1";
/// The `tool` label of the metrics series and the `implementation` of the summary.
pub const IMPLEMENTATION: &str = "rust-api";

/// The largest symbol set, batch and number of instances that the tool accepts; the symbol set is also the largest
/// that the tool generates, as the C tool limits it. A batch shares one time, so it holds at most 2^22 events: the
/// 22-bit sequence must not wrap inside one batch.
pub const MAX_SYMBOLS: i64 = 100_000_000;
const MAX_BATCH: i64 = 1 << 22;
const MAX_INSTANCES: i64 = 1_000;

/// The synopsis of the tool, the usage line of its help screen.
pub const SYNOPSIS: &str = "NetTest p|c <address> -S <symbols> [options]";

/// The lines of the usage text after the synopsis, without the closing line `spec_version`.
pub const OPTION_LINES: &[&str] = &[
    "  p                    publish generated events",
    "  c                    receive them and report the end-to-end lag",
    "  <address>            host:port to connect to, :port to listen on",
    "  -S <total>           the symbols: the <total> generated symbols of QD NetTest",
    "  -S <total>@<file>    the first <total> symbols of the IPF file, in file order",
    "  -C <n>               the number of instances, default 1 (not above 1 with a :port address);",
    "                       on c the instances share the symbols",
    "  -r, --rate-limit */<rate>",
    "                       p: the total rate in records per second, with k, M or G (*/1M)",
    "  -s, --stat <period>  the statistics period, default 10s (ms, s or m)",
    "  -n, --name <name>    the name in the statistics lines, default NetTest",
    "  -c, --collector <contract>",
    "                       stream or ticker: the contract that the consumer receives and the",
    "                       default roles (none: PUBLISHER and STREAM_FEED)",
    "  --event <type>       Trade (default) or TimeAndSale",
    "  --role <role>        PUBLISHER or STREAM_PUBLISHER on p, FEED or STREAM_FEED on c",
    "  --batch <n>          p: the events per publishing call, default 100",
    "  --reuse              p: republish the pre-built events in place instead of copying them",
    "  --duration <time>    end the run after the time (ms, s or m); Ctrl+C ends it at once",
    "  --warmup <time>      leave the first <time> out of the e2e lag of the summary; on c it starts",
    "                       at the first record, on p at the first batch",
    "  --summary <file>     write the JSON summary of the run (nettest-summary/1) at the end",
    "  --metrics-port [<host>:]<port>",
    "                       serve /metrics in the Prometheus text format; the host defaults to",
    "                       127.0.0.1, port 0 takes a free port",
    "  -D<key>=<value>      set a system property before the first endpoint is created",
    "                       (dxscheme.nanoTime also goes to the builder of every endpoint, default true)",
    "  -Xmx<size>, -Xms<size>, -Xmn<size>, -Xss<size>",
    "                       the maximum, initial and young generation heap sizes and the thread stack size of the",
    "                       isolate (512m, 1g)",
    "  -XX:<option>         an option of the isolate (for example -XX:+PrintGC -XX:+PrintGCSummary)",
    "  --help               print this text and exit",
    "",
    "A long option also takes its value as --option=value.",
    "",
];

/// The usage text with the line `spec_version`, ending with a line break.
pub fn usage_text() -> String {
    let mut text = format!("Usage: Tools {SYNOPSIS}\n\n");
    for line in OPTION_LINES {
        text.push_str(line);
        text.push('\n');
    }
    text.push_str(&format!("spec_version {SPEC_VERSION}\n"));
    text
}

/// The mode of the run: the first argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Producer,
    Consumer,
}

/// The event type that the instances publish or receive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    Trade,
    TimeAndSale,
}

impl EventKind {
    /// The name as the specification writes it.
    pub fn name(self) -> &'static str {
        match self {
            EventKind::Trade => "Trade",
            EventKind::TimeAndSale => "TimeAndSale",
        }
    }
}

/// The contract of `-c`: what the consumer receives and, through it, the default roles of the endpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Collector {
    Stream,
    Ticker,
}

impl Collector {
    /// The name as the specification writes it.
    pub fn name(self) -> &'static str {
        match self {
            Collector::Stream => "stream",
            Collector::Ticker => "ticker",
        }
    }
}

/// The name of a role as the specification writes it.
pub fn role_name(role: Role) -> &'static str {
    match role {
        Role::Feed => "FEED",
        Role::OnDemandFeed => "ON_DEMAND_FEED",
        Role::StreamFeed => "STREAM_FEED",
        Role::Publisher => "PUBLISHER",
        Role::StreamPublisher => "STREAM_PUBLISHER",
        Role::LocalHub => "LOCAL_HUB",
        _ => "UNKNOWN",
    }
}

/// The property of the nanosecond part of the event time, which every endpoint builder carries.
const NANO_TIME_PROPERTY: &str = "dxscheme.nanoTime";

/// The parsed and validated command line.
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    pub mode: Mode,
    /// `--help` was given: the usage goes to stdout and the exit code is 0; the other fields are not validated then.
    pub help: bool,
    /// `host:port` to connect, `:port` to listen.
    pub address: String,
    /// The `<total>` of `-S`.
    pub total_symbols: i64,
    /// The `@<file>` of `-S`; `None` when the symbols are generated.
    pub symbols_file: Option<String>,
    /// `-C`.
    pub instances: usize,
    /// The total rate of all instances (`-r */<rate>`); `None` when the generators run flat out.
    pub records_per_second: Option<i64>,
    /// `-s` in milliseconds.
    pub period_millis: i64,
    /// `-n`: the name in the statistics line and the base of the endpoint names.
    pub name: String,
    /// `--batch`.
    pub batch: usize,
    /// `--event`.
    pub event: EventKind,
    /// `-c`; `None` without it.
    pub collector: Option<Collector>,
    /// `--role`; `None` when `-c` or the mode selects it (see `role()`).
    pub role: Option<Role>,
    /// `--reuse`.
    pub reuse: bool,
    /// `--duration` in milliseconds; `None` when only a signal ends the run.
    pub duration_millis: Option<i64>,
    /// `--warmup` in milliseconds.
    pub warmup_millis: Option<i64>,
    /// `--summary`; the directory of the file was writable when the command line was parsed.
    pub summary_file: Option<String>,
    /// The host of `--metrics-port`, `127.0.0.1` by default; an IPv6 literal keeps its brackets.
    pub metrics_host: String,
    /// The port of `--metrics-port`; `None` without the option, 0 takes a free port.
    pub metrics_port: Option<u16>,
    /// The arguments after the tool name as they were given.
    pub arguments: Vec<String>,
    /// `-D<key>=<value>` in command-line order.
    pub properties: Vec<(String, String)>,
    /// `-Xmx`, `-Xms`, `-Xmn`, `-Xss` and `-XX:` as they are, in command-line order.
    pub isolate_arguments: Vec<String>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            mode: Mode::Producer,
            help: false,
            address: String::new(),
            total_symbols: 0,
            symbols_file: None,
            instances: 1,
            records_per_second: None,
            period_millis: 10_000,
            name: "NetTest".to_owned(),
            batch: 100,
            event: EventKind::Trade,
            collector: None,
            role: None,
            reuse: false,
            duration_millis: None,
            warmup_millis: None,
            summary_file: None,
            metrics_host: "127.0.0.1".to_owned(),
            metrics_port: None,
            arguments: Vec::new(),
            properties: Vec::new(),
            isolate_arguments: Vec::new(),
        }
    }
}

impl Options {
    /// The role of the endpoints: `--role` when given, else the role that `-c` selects for the mode.
    pub fn role(&self) -> Role {
        if let Some(role) = self.role {
            return role;
        }
        match (self.mode, self.collector) {
            (Mode::Producer, Some(Collector::Stream)) => Role::StreamPublisher,
            (Mode::Producer, _) => Role::Publisher,
            (Mode::Consumer, Some(Collector::Ticker)) => Role::Feed,
            (Mode::Consumer, _) => Role::StreamFeed,
        }
    }

    /// Whether the consumer receives conflated events: the role `FEED` with `Trade`, which it receives through the
    /// ticker contract. Always false on `p`.
    pub fn is_conflated(&self) -> bool {
        self.mode == Mode::Consumer && self.role() == Role::Feed && self.event == EventKind::Trade
    }

    /// The value of `dxscheme.nanoTime` for the endpoint builders: the last `-Ddxscheme.nanoTime` of the command line,
    /// `true` without one. The system property alone does not reach the endpoints of the stream roles.
    pub fn nano_time(&self) -> &str {
        self.properties
            .iter()
            .rev()
            .find(|(key, _)| key == NANO_TIME_PROPERTY)
            .map_or("true", |(_, value)| value)
    }

    /// Whether the endpoints carry the nanosecond part: `nano_time()` is `true`, in any case, as Java reads it.
    pub fn is_nano_time_on(&self) -> bool {
        self.nano_time().eq_ignore_ascii_case("true")
    }
}

/// Whether the address makes an endpoint listen (it starts with `:`).
pub fn is_server_address(address: &str) -> bool {
    address.starts_with(':')
}

fn is_digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())
}

/// A positive decimal integer of at most `limit`.
fn parse_positive(text: &str, limit: i64) -> Option<i64> {
    if !is_digits(text) || text.len() > 18 {
        return None;
    }
    let value: i64 = text.parse().ok()?;
    (1..=limit).contains(&value).then_some(value)
}

/// A duration with the suffix `ms`, `s` or `m` (`500ms`, `10s`, `2m`) in milliseconds; `None` for a missing suffix, a
/// non-positive value, more than 15 digits or an overflow.
fn parse_duration(text: &str) -> Option<i64> {
    let (number, multiplier) = if text.len() > 2 && text.ends_with("ms") {
        (&text[..text.len() - 2], 1)
    } else if text.len() > 1 && text.ends_with('s') {
        (&text[..text.len() - 1], 1_000)
    } else if text.len() > 1 && text.ends_with('m') {
        (&text[..text.len() - 1], 60_000)
    } else {
        return None;
    };
    if !is_digits(number) || number.len() > 15 {
        return None;
    }
    let value: i64 = number.parse().ok()?;
    if value < 1 || value > i64::MAX / multiplier {
        return None;
    }
    Some(value * multiplier)
}

/// A rate with the optional suffix `k`, `M` or `G` (`500k`, `1M`, `2.5M`) in records per second; `None` below 1 record
/// per second or above 10^12.
fn parse_rate(text: &str) -> Option<i64> {
    let (number, multiplier) = match text.as_bytes().last()? {
        b'k' => (&text[..text.len() - 1], 1e3),
        b'M' => (&text[..text.len() - 1], 1e6),
        b'G' => (&text[..text.len() - 1], 1e9),
        _ => (text, 1.0),
    };
    // Digits with at most one dot: no sign, no exponent, no spaces.
    if number.is_empty() || number.starts_with('.') || number.ends_with('.') {
        return None;
    }
    if !number.bytes().all(|byte| byte == b'.' || byte.is_ascii_digit()) || number.matches('.').count() > 1 {
        return None;
    }
    let rate = number.parse::<f64>().ok()? * multiplier;
    if !(1.0..=1e12).contains(&rate) {
        return None;
    }
    Some(rate.round() as i64)
}

/// Whether a file can be created in the directory of the path (the current directory for a bare file name) and the
/// path is not a directory. A probe file next to the summary is the only check of the permissions that works alike
/// everywhere.
fn is_writable_directory_of(file: &str) -> bool {
    let path = Path::new(file);
    let directory = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    if !directory.is_dir() || path.is_dir() {
        return false;
    }
    let mut probe = path.as_os_str().to_owned();
    probe.push(".probe");
    let probe = PathBuf::from(probe);
    let existed = probe.exists();
    let writable = OpenOptions::new().create(true).append(true).open(&probe).is_ok();
    if writable && !existed {
        let _ = fs::remove_file(&probe);
    }
    writable
}

/// The options of the specification by their names; the long names of the short options are aliases.
#[derive(Debug, Clone, Copy)]
enum Flag {
    Symbols,
    Instances,
    Rate,
    Stat,
    Name,
    Collector,
    Event,
    Batch,
    Reuse,
    Role,
    Duration,
    Warmup,
    Summary,
    MetricsPort,
    Legacy,
    Help,
    Wildcard,
}

struct Spec {
    name: &'static str,
    flag: Flag,
    /// Takes a value: the next argument, or the text after `=` of a long option.
    has_value: bool,
    /// An option of the mode `p` only, rejected on `c`.
    producer_only: bool,
}

const fn spec(name: &'static str, flag: Flag, has_value: bool, producer_only: bool) -> Spec {
    Spec {
        name,
        flag,
        has_value,
        producer_only,
    }
}

const OPTIONS: &[Spec] = &[
    spec("-S", Flag::Symbols, true, false),
    spec("-C", Flag::Instances, true, false),
    spec("-r", Flag::Rate, true, true),
    spec("--rate-limit", Flag::Rate, true, true),
    spec("-s", Flag::Stat, true, false),
    spec("--stat", Flag::Stat, true, false),
    spec("-n", Flag::Name, true, false),
    spec("--name", Flag::Name, true, false),
    spec("-c", Flag::Collector, true, false),
    spec("--collector", Flag::Collector, true, false),
    spec("--event", Flag::Event, true, false),
    spec("--batch", Flag::Batch, true, true),
    spec("--reuse", Flag::Reuse, false, true),
    spec("--role", Flag::Role, true, false),
    spec("--duration", Flag::Duration, true, false),
    spec("--legacy", Flag::Legacy, false, true),
    spec("--warmup", Flag::Warmup, true, false),
    spec("--summary", Flag::Summary, true, false),
    spec("--metrics-port", Flag::MetricsPort, true, false),
    spec("--help", Flag::Help, false, false),
    spec("-w", Flag::Wildcard, false, false),
    spec("--wildcard", Flag::Wildcard, false, false),
];

/// The state of one parsing.
#[derive(Default)]
struct Parser {
    options: Options,
    address_seen: bool,
    symbols_seen: bool,
}

impl Parser {
    fn producer(&self) -> bool {
        self.options.mode == Mode::Producer
    }

    fn parse_symbols(&mut self, flag: &str, value: &str) -> Result<(), String> {
        let at = value.find('@');
        let head = &value[..at.unwrap_or(value.len())];
        if head.contains('{') {
            return Err(format!(
                "unsupported QD flag: the symbol length '{{len,max}}' in '{flag} {value}'"
            ));
        }
        if head.contains('/') {
            return Err(format!(
                "unsupported QD flag: the symbol split '/r' or '/s' in '{flag} {value}'"
            ));
        }
        let Some(total) = parse_positive(head, MAX_SYMBOLS) else {
            return Err(format!(
                "{flag} needs <total> or <total>@<file>, where <total> is a number from 1 to {MAX_SYMBOLS}: '{value}'"
            ));
        };
        self.options.total_symbols = total;
        self.options.symbols_file = None;
        if let Some(at) = at {
            let file = &value[at + 1..];
            if file.is_empty() {
                return Err(format!("{flag} has no file name after '@': '{value}'"));
            }
            self.options.symbols_file = Some(file.to_owned());
        }
        self.symbols_seen = true;
        Ok(())
    }

    fn parse_rate_option(&mut self, flag: &str, value: &str) -> Result<(), String> {
        if value.contains(';') {
            return Err(format!(
                "unsupported QD flag: the rate limiter bucket ';<time>' in '{flag} {value}'"
            ));
        }
        if value.len() < 3 || !value.starts_with("*/") {
            return Err(format!("{flag} needs */<rate> (for example */500k): '{value}'"));
        }
        let Some(rate) = parse_rate(&value[2..]) else {
            return Err(format!(
                "{flag} has a bad rate (a positive number with an optional k, M or G): '{value}'"
            ));
        };
        self.options.records_per_second = Some(rate);
        Ok(())
    }

    /// `[<host>:]<port>`, the port from 0 to 65535; the host is the text before the last `:` and must not be empty.
    fn parse_metrics_port(&mut self, flag: &str, value: &str) -> Result<(), String> {
        let (host, port) = match value.rfind(':') {
            Some(colon) => (&value[..colon], &value[colon + 1..]),
            None => ("127.0.0.1", value),
        };
        let port = if host.is_empty() || !is_digits(port) || port.len() > 5 {
            None
        } else {
            port.parse::<u16>().ok()
        };
        let Some(port) = port else {
            return Err(format!(
                "{flag} needs [<host>:]<port>, where <port> is a number from 0 to 65535: '{value}'"
            ));
        };
        self.options.metrics_host = host.to_owned();
        self.options.metrics_port = Some(port);
        Ok(())
    }

    fn parse_collector(&mut self, flag: &str, value: &str) -> Result<(), String> {
        self.options.collector = Some(match value {
            "stream" => Collector::Stream,
            "ticker" => Collector::Ticker,
            "history" => return Err(format!("{flag} history is reserved")),
            _ => return Err(format!("{flag} is stream or ticker: '{value}'")),
        });
        Ok(())
    }

    fn parse_role(&mut self, flag: &str, value: &str) -> Result<(), String> {
        let (first, second) = if self.producer() {
            (Role::Publisher, Role::StreamPublisher)
        } else {
            (Role::Feed, Role::StreamFeed)
        };
        self.options.role = Some(if value == role_name(first) {
            first
        } else if value == role_name(second) {
            second
        } else {
            return Err(format!(
                "{flag} is {} or {}: '{value}'",
                role_name(first),
                role_name(second)
            ));
        });
        Ok(())
    }

    fn parse_property(&mut self, argument: &str) -> Result<(), String> {
        match argument.find('=') {
            Some(equals) if equals >= 3 => {
                self.options
                    .properties
                    .push((argument[2..equals].to_owned(), argument[equals + 1..].to_owned()));
                Ok(())
            }
            _ => Err(format!("-D needs <key>=<value>: '{argument}'")),
        }
    }

    /// The isolate checks the `-XX:` options and the values of `-Xmx`, `-Xms`, `-Xmn` and `-Xss`, but ignores the
    /// other `-X` arguments, so that a typo there would go unnoticed: they are rejected here.
    fn parse_isolate_argument(&mut self, argument: &str) -> Result<(), String> {
        let has_value = argument.len() > 4;
        let size = has_value
            && ["-Xmx", "-Xms", "-Xmn", "-Xss"]
                .iter()
                .any(|prefix| argument.starts_with(prefix));
        let runtime_option = has_value && argument.starts_with("-XX:");
        if !size && !runtime_option {
            return Err(format!(
                "{argument} is not an option of the isolate, which takes -Xmx<size>, -Xms<size>, -Xmn<size>, \
                 -Xss<size> and -XX:<option>"
            ));
        }
        self.options.isolate_arguments.push(argument.to_owned());
        Ok(())
    }

    /// Applies one option with its value (empty for a switch).
    fn apply(&mut self, flag_spec: &Spec, flag: &str, value: &str) -> Result<(), String> {
        match flag_spec.flag {
            Flag::Symbols => self.parse_symbols(flag, value),
            Flag::Instances => {
                self.options.instances = parse_number(flag, value, MAX_INSTANCES)? as usize;
                Ok(())
            }
            Flag::Rate => self.parse_rate_option(flag, value),
            Flag::Stat => {
                self.options.period_millis = parse_time(flag, value)?;
                Ok(())
            }
            Flag::Name => {
                if value.is_empty() {
                    return Err(format!("{flag} needs a non-empty name"));
                }
                self.options.name = value.to_owned();
                Ok(())
            }
            Flag::Collector => self.parse_collector(flag, value),
            Flag::Event => {
                self.options.event = match value {
                    "Trade" => EventKind::Trade,
                    "TimeAndSale" => EventKind::TimeAndSale,
                    _ => return Err(format!("{flag} is Trade or TimeAndSale: '{value}'")),
                };
                Ok(())
            }
            Flag::Batch => {
                self.options.batch = parse_number(flag, value, MAX_BATCH)? as usize;
                Ok(())
            }
            Flag::Reuse => {
                self.options.reuse = true;
                Ok(())
            }
            Flag::Role => self.parse_role(flag, value),
            Flag::Duration => {
                self.options.duration_millis = Some(parse_time(flag, value)?);
                Ok(())
            }
            Flag::Warmup => {
                self.options.warmup_millis = Some(parse_time(flag, value)?);
                Ok(())
            }
            Flag::Summary => {
                if value.is_empty() || !is_writable_directory_of(value) {
                    return Err(format!("{flag} cannot write to the directory of '{value}'"));
                }
                self.options.summary_file = Some(value.to_owned());
                Ok(())
            }
            Flag::MetricsPort => self.parse_metrics_port(flag, value),
            Flag::Legacy => Err(format!(
                "unsupported option: {flag} (the Rust API has no deprecated publishing function)"
            )),
            Flag::Help => {
                self.options.help = true;
                Ok(())
            }
            Flag::Wildcard => Err(format!("unsupported QD flag: {flag} (the wildcard subscription)")),
        }
    }

    /// The rules between the options, after all of them are read (a repeated option takes its last value).
    fn validate(&self) -> Result<(), String> {
        let options = &self.options;
        if !self.address_seen || options.address.is_empty() {
            return Err("the address is missing".to_owned());
        }
        if !self.symbols_seen {
            return Err("-S is missing".to_owned());
        }
        if options.instances > 1 && is_server_address(&options.address) {
            return Err(format!(
                "-C {} cannot be used with the server address '{}': every instance would bind the same port",
                options.instances, options.address
            ));
        }
        let event = options.event.name();
        if options.collector == Some(Collector::Ticker) && options.event != EventKind::Trade {
            return Err(format!("-c ticker needs --event Trade: {event} has no ticker contract"));
        }
        // Without --role, -c selects a role that receives its contract; without -c, either role of the mode is valid.
        let (Some(collector), Some(role)) = (options.collector, options.role) else {
            return Ok(());
        };
        match collector {
            // The ticker contract needs the roles that write and read it.
            Collector::Ticker if matches!(role, Role::StreamPublisher | Role::StreamFeed) => Err(format!(
                "--role {} conflicts with -c ticker for {event}",
                role_name(role)
            )),
            // FEED receives Trade through the ticker contract, not the stream contract of -c stream.
            Collector::Stream if role == Role::Feed && options.event == EventKind::Trade => Err(format!(
                "--role {} conflicts with -c stream for {event}",
                role_name(role)
            )),
            _ => Ok(()),
        }
    }

    fn run(&mut self, args: &[String]) -> Result<(), String> {
        let Some(mode) = args.first() else {
            return Err("no mode".to_owned());
        };
        if mode == "--help" {
            self.options.help = true;
            return Ok(());
        }
        self.options.mode = match mode.as_str() {
            "p" => Mode::Producer,
            "c" => Mode::Consumer,
            _ => return Err(format!("unknown mode '{mode}'; the mode is p or c")),
        };
        self.options.arguments = args.to_vec();

        let mut index = 1;
        while index < args.len() {
            let argument = &args[index];
            index += 1;
            if argument.starts_with("-D") {
                self.parse_property(argument)?;
                continue;
            }
            if argument.starts_with("-X") {
                self.parse_isolate_argument(argument)?;
                continue;
            }
            if argument.len() < 2 || !argument.starts_with('-') {
                if self.address_seen {
                    return Err(format!(
                        "unexpected argument '{argument}' after the address '{}'",
                        self.options.address
                    ));
                }
                self.options.address = argument.clone();
                self.address_seen = true;
                continue;
            }

            // A long option may carry its value after `=`.
            let (flag, inline_value) = match argument.split_once('=') {
                Some((flag, value)) if argument.starts_with("--") => (flag, Some(value)),
                _ => (argument.as_str(), None),
            };
            let Some(flag_spec) = OPTIONS.iter().find(|spec| spec.name == flag) else {
                return Err(format!("unknown flag {flag}"));
            };
            if flag_spec.producer_only && !self.producer() {
                return Err(format!("{flag} is an option of the mode p"));
            }
            let value = if flag_spec.has_value {
                match inline_value {
                    Some(value) => value,
                    None if index < args.len() => {
                        index += 1;
                        args[index - 1].as_str()
                    }
                    None => return Err(format!("missing value of {flag}")),
                }
            } else if inline_value.is_some() {
                return Err(format!("{flag} takes no value: '{argument}'"));
            } else {
                ""
            };
            self.apply(flag_spec, flag, value)?;
            // --help ends the parsing: the usage is printed whatever follows.
            if self.options.help {
                return Ok(());
            }
        }
        self.validate()
    }
}

fn parse_number(flag: &str, value: &str, limit: i64) -> Result<i64, String> {
    parse_positive(value, limit).ok_or_else(|| format!("{flag} needs a number from 1 to {limit}: '{value}'"))
}

fn parse_time(flag: &str, value: &str) -> Result<i64, String> {
    parse_duration(value)
        .ok_or_else(|| format!("{flag} needs a positive time with ms, s or m (for example 10s): '{value}'"))
}

/// Parses the arguments after `NetTest`, the mode first; the error is the message of the first failure. With `--help`
/// right after the mode the other arguments are not read.
pub fn parse(args: &[String]) -> Result<Options, String> {
    let mut parser = Parser::default();
    parser.run(args)?;
    Ok(parser.options)
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;
    use crate::net_test::kit::kit_json;

    fn parse_args(args: &[&str]) -> Result<Options, String> {
        parse(&args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>())
    }

    fn options(args: &[&str]) -> Options {
        parse_args(args).unwrap_or_else(|error| panic!("{args:?}: {error}"))
    }

    #[track_caller]
    fn rejected(args: &[&str], error: &str) {
        match parse_args(args) {
            Ok(options) => panic!("{args:?} accepted: {options:?}"),
            Err(found) => assert_eq!(found, error, "{args:?}"),
        }
    }

    /// The setting of the kit by its name in the config of the summary file.
    fn setting_of(options: &Options, key: &str) -> Value {
        let producer = options.mode == Mode::Producer;
        let on_producer = |value: Value| if producer { value } else { Value::Null };
        match key {
            "side" => Value::from(if producer { "producer" } else { "consumer" }),
            "address" => Value::from(options.address.as_str()),
            "symbols" => Value::from(options.total_symbols),
            "symbols_source" => Value::from(if options.symbols_file.is_some() {
                "file"
            } else {
                "generated"
            }),
            "instances" => Value::from(options.instances),
            "rate_limit" => on_producer(options.records_per_second.map_or(Value::Null, Value::from)),
            "batch" => on_producer(Value::from(options.batch)),
            "event" => Value::from(options.event.name()),
            "collector" => options.collector.map_or(Value::Null, |c| Value::from(c.name())),
            "role" => Value::from(role_name(options.role())),
            "reuse" => on_producer(Value::from(options.reuse)),
            "period_ms" => Value::from(options.period_millis),
            "warmup_ms" => options.warmup_millis.map_or(Value::Null, Value::from),
            "duration_ms" => options.duration_millis.map_or(Value::Null, Value::from),
            "name" => Value::from(options.name.as_str()),
            "conflated" => Value::from(options.is_conflated()),
            "isolate_arguments" => Value::from(options.isolate_arguments.clone()),
            other => panic!("unknown setting {other}"),
        }
    }

    /// The optional features of this tool: it has the isolate arguments and lacks the legacy publishing function.
    const FEATURES: &[&str] = &["isolate"];

    fn needs_absent_feature(entry: &Value) -> bool {
        entry["requires"].as_array().is_some_and(|requires| {
            requires.iter().any(|feature| {
                let feature = feature.as_str().unwrap();
                match feature.strip_prefix('!') {
                    Some(absent) => FEATURES.contains(&absent),
                    None => !FEATURES.contains(&feature),
                }
            })
        })
    }

    #[test]
    fn the_parser_gives_the_settings_and_the_errors_of_cli_vectors() {
        let vectors = kit_json("cli-vectors.json");
        let entries = vectors["vectors"].as_array().unwrap();
        assert!(!entries.is_empty(), "cli-vectors.json has no vectors");
        let (mut checked, mut skipped) = (0, 0);
        for entry in entries {
            if needs_absent_feature(entry) {
                skipped += 1;
                continue;
            }
            let args: Vec<String> = entry["args"]
                .as_array()
                .unwrap()
                .iter()
                .map(|argument| argument.as_str().unwrap().to_owned())
                .collect();
            let command_line = args.join(" ");
            // The tool prints the usage and exits with 2 without arguments, before any parsing.
            let result = if args.is_empty() {
                Err("no mode".to_owned())
            } else {
                parse(&args)
            };
            if entry["exit"] == 0 {
                let options = result.unwrap_or_else(|error| panic!("{command_line}: {error}"));
                match entry.get("stdout_contains") {
                    Some(line) => {
                        assert!(options.help, "{command_line}");
                        assert!(usage_text().contains(line.as_str().unwrap()), "{command_line}");
                    }
                    None => assert!(!options.help, "{command_line}"),
                }
                for (key, expected) in entry.get("settings").and_then(Value::as_object).into_iter().flatten() {
                    assert_eq!(
                        &setting_of(&options, key),
                        expected,
                        "{command_line}: the setting {key}"
                    );
                }
            } else {
                let error = result.expect_err(&command_line);
                if let Some(text) = entry.get("error") {
                    assert_eq!(error, text.as_str().unwrap(), "{command_line}");
                } else if let Some(prefix) = entry.get("error_prefix") {
                    assert!(error.starts_with(prefix.as_str().unwrap()), "{command_line}: {error}");
                }
            }
            checked += 1;
        }
        println!("cli-vectors.json: {checked} checked, {skipped} skipped");
        assert!(checked >= 70, "only {checked} vectors checked, {skipped} skipped");
    }

    #[test]
    fn the_defaults_of_a_minimal_command_line() {
        let parsed = options(&["p", ":7000", "-S", "10"]);
        assert_eq!(
            parsed,
            Options {
                mode: Mode::Producer,
                address: ":7000".to_owned(),
                total_symbols: 10,
                arguments: vec!["p".to_owned(), ":7000".to_owned(), "-S".to_owned(), "10".to_owned()],
                ..Options::default()
            }
        );
        assert_eq!(parsed.role(), Role::Publisher);
        assert_eq!(parsed.metrics_host, "127.0.0.1");
        assert_eq!(parsed.nano_time(), "true");
        assert!(parsed.is_nano_time_on());
    }

    #[test]
    fn system_properties_keep_the_order_and_split_at_the_first_equals_sign() {
        let parsed = options(&[
            "p",
            ":7000",
            "-Da=1",
            "-S",
            "10",
            "-Dk=a=b",
            "-Dempty=",
            "-Da=2",
            "-Xmx64m",
            "-XX:+PrintGC",
            "-Xss1m",
        ]);
        let expected: Vec<(String, String)> = [("a", "1"), ("k", "a=b"), ("empty", ""), ("a", "2")]
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        assert_eq!(parsed.properties, expected);
        assert_eq!(parsed.isolate_arguments, ["-Xmx64m", "-XX:+PrintGC", "-Xss1m"]);
    }

    #[test]
    fn the_last_nano_time_property_goes_to_the_builders() {
        let nano = |properties: &[&str]| {
            let mut args = vec!["p", ":7000", "-S", "1"];
            args.extend_from_slice(properties);
            let parsed = options(&args);
            (parsed.nano_time().to_owned(), parsed.is_nano_time_on())
        };
        assert_eq!(nano(&[]), ("true".to_owned(), true));
        assert_eq!(nano(&["-Ddxscheme.nanoTime=false"]), ("false".to_owned(), false));
        assert_eq!(
            nano(&["-Ddxscheme.nanoTime=false", "-Ddxscheme.nanoTime=TRUE"]),
            ("TRUE".to_owned(), true)
        );
        assert_eq!(
            nano(&["-Ddxscheme.nanoTime=true", "-Ddxscheme.nanoTime=no"]),
            ("no".to_owned(), false)
        );
        assert_eq!(nano(&["-Ddxscheme.nanoTime=tru"]), ("tru".to_owned(), false));
    }

    #[test]
    fn options_come_before_or_after_the_address() {
        let before = options(&["p", "-S", "10", "-r", "*/1k", "--batch=5", "127.0.0.1:7000", "--reuse"]);
        let after = options(&["p", "127.0.0.1:7000", "-S", "10", "-r", "*/1k", "--batch=5", "--reuse"]);
        assert_eq!(before.address, "127.0.0.1:7000");
        assert_eq!(
            Options {
                arguments: Vec::new(),
                ..before
            },
            Options {
                arguments: Vec::new(),
                ..after
            }
        );
        // A lone dash is not an option: it is the address.
        assert_eq!(options(&["p", "-", "-S", "1"]).address, "-");
    }

    #[test]
    fn a_windows_path_with_a_drive_stays_in_the_symbol_file() {
        for (value, file) in [
            ("10@C:/data/some/symbols.ipf", "C:/data/some/symbols.ipf"),
            ("10@C:\\data\\symbols.ipf", "C:\\data\\symbols.ipf"),
            ("10@../dir/s.ipf", "../dir/s.ipf"),
            // The first @ splits; a slash after it is part of the path.
            ("10@a@b/c.ipf", "a@b/c.ipf"),
        ] {
            let parsed = options(&["p", ":1", "-S", value]);
            assert_eq!(parsed.total_symbols, 10, "{value}");
            assert_eq!(parsed.symbols_file.as_deref(), Some(file), "{value}");
        }
        // A later -S without a file drops the file of an earlier one.
        assert_eq!(options(&["p", ":1", "-S", "5@a.ipf", "-S", "7"]).symbols_file, None);
    }

    #[test]
    fn an_option_of_the_mode_p_on_c_names_the_option() {
        for flag in ["-r", "--rate-limit", "--batch", "--reuse", "--legacy"] {
            let expected = format!("{flag} is an option of the mode p");
            let value = if flag == "--reuse" || flag == "--legacy" {
                Vec::new()
            } else {
                vec!["*/1k"]
            };
            let mut args = vec!["c", "127.0.0.1:7000", "-S", "10", flag];
            args.extend(&value);
            rejected(&args, &expected);
            // The mode is checked before the value: no value gives the same line.
            rejected(&["c", "127.0.0.1:7000", "-S", "10", flag], &expected);
            // A long option with a value after `=` gives the same line; a short one has no such form.
            if flag.starts_with("--") {
                rejected(&["c", "127.0.0.1:7000", &format!("{flag}=1")], &expected);
            }
        }
        // The same options are valid on p.
        let parsed = options(&[
            "p",
            "127.0.0.1:7000",
            "-S",
            "10",
            "--rate-limit",
            "*/1k",
            "--batch",
            "7",
        ]);
        assert_eq!((parsed.records_per_second, parsed.batch), (Some(1000), 7));
    }

    #[test]
    fn legacy_is_unsupported_on_p() {
        let error = parse_args(&["p", ":7000", "-S", "10", "--legacy"]).unwrap_err();
        assert!(error.starts_with("unsupported option: --legacy"), "{error}");
        let error = parse_args(&["p", ":7000", "--legacy=1"]).unwrap_err();
        assert_eq!(error, "--legacy takes no value: '--legacy=1'");
    }

    #[test]
    fn help_stops_the_parsing() {
        assert!(options(&["--help"]).help);
        assert!(options(&["--help", "--no-such-flag"]).help);
        assert!(options(&["c", "--help", "--no-such-flag"]).help);
        // Nothing is validated after it: no address and no symbols.
        assert!(options(&["p", "--help"]).help);
        assert!(options(&["p", "--help", "-S", "0", "extra"]).help);
        // An error before it wins.
        rejected(&["p", "--no-such-flag", "--help"], "unknown flag --no-such-flag");
        rejected(
            &["p", "-S", "0", "--help"],
            "-S needs <total> or <total>@<file>, where <total> is a number from 1 to 100000000: '0'",
        );
        // A switch takes no value.
        rejected(&["p", "--help=yes"], "--help takes no value: '--help=yes'");
        // Only the first argument may be --help without a mode.
        rejected(&["x", "--help"], "unknown mode 'x'; the mode is p or c");
        assert_eq!(parse_args(&[]), Err("no mode".to_owned()));
    }

    #[test]
    fn the_first_error_wins_in_the_order_of_the_parser() {
        rejected(&["p", ":7000", "--unknown", "-S", "0"], "unknown flag --unknown");
        // -D and -X are read by their prefix before the address and the option table.
        rejected(&["p", "-Dnovalue", "--unknown"], "-D needs <key>=<value>: '-Dnovalue'");
        rejected(
            &["p", "-Xfoo", "--unknown"],
            "-Xfoo is not an option of the isolate, which takes -Xmx<size>, -Xms<size>, -Xmn<size>, -Xss<size> and \
             -XX:<option>",
        );
        // The value of an option is checked at its position, the rules between the options after the last argument.
        rejected(
            &["p", ":7000", "-C", "0", "-S", "0"],
            "-C needs a number from 1 to 1000: '0'",
        );
        rejected(&["p", ":7000", "-C", "2"], "-S is missing");
        rejected(&["p", "-S", "10"], "the address is missing");
        rejected(&["p", "", "-S", "10"], "the address is missing");
        rejected(&["p", ":7000", "-S"], "missing value of -S");
        // The value is the next argument as it is, also when it looks like an option.
        rejected(
            &["p", ":7000", "-S", "-C"],
            "-S needs <total> or <total>@<file>, where <total> is a number from 1 to 100000000: '-C'",
        );
        rejected(&["p", ":7000", "-S", "10", "-S=5"], "unknown flag -S=5");
        rejected(&["p", ":7000", "-S", "10", "-S5"], "unknown flag -S5");
        rejected(&["p", ":7000", "-S", "10", "-5"], "unknown flag -5");
        rejected(
            &["p", ":7000", "-S", "10", "--name=x=y", "--reuse=1"],
            "--reuse takes no value: '--reuse=1'",
        );
        assert_eq!(options(&["p", ":7000", "-S", "10", "--name=x=y"]).name, "x=y");
    }

    #[test]
    fn the_option_texts_carry_the_flag_as_typed() {
        rejected(
            &["p", ":7000", "-S", "10", "--stat", "5h"],
            "--stat needs a positive time with ms, s or m (for example 10s): '5h'",
        );
        rejected(
            &["p", ":7000", "-S", "10", "--rate-limit=*/0"],
            "--rate-limit has a bad rate (a positive number with an optional k, M or G): '*/0'",
        );
        rejected(&["p", ":7000", "-S", "10", "--name="], "--name needs a non-empty name");
        rejected(
            &["p", ":7000", "-S", "10", "--collector=x"],
            "--collector is stream or ticker: 'x'",
        );
        rejected(
            &["p", ":7000", "-S", "10", "--collector", "history"],
            "--collector history is reserved",
        );
        rejected(
            &["p", ":7000", "-S", "10", "-r", "*/"],
            "-r needs */<rate> (for example */500k): '*/'",
        );
        rejected(
            &["p", ":7000", "-S", "10", "--warmup", "0s"],
            "--warmup needs a positive time with ms, s or m (for example 10s): '0s'",
        );
        rejected(
            &["p", ":7000", "-w"],
            "unsupported QD flag: -w (the wildcard subscription)",
        );
        rejected(
            &["c", "a:1", "-S", "1", "--wildcard"],
            "unsupported QD flag: --wildcard (the wildcard subscription)",
        );
        rejected(
            &["p", ":7000", "-S", "10{3}"],
            "unsupported QD flag: the symbol length '{len,max}' in '-S 10{3}'",
        );
        rejected(
            &["p", ":7000", "-S", "10/s5"],
            "unsupported QD flag: the symbol split '/r' or '/s' in '-S 10/s5'",
        );
        // A slash after the @ belongs to the file name.
        assert_eq!(options(&["p", ":7000", "-S", "10@dir/file.ipf"]).total_symbols, 10);
        for argument in [
            "-X", "-XX", "-XX:", "-Xmx", "-Xss", "-Xfoo", "-Xsz64k", "-Xnm2m", "-XXfoo",
        ] {
            let error = parse_args(&["p", ":7000", "-S", "10", argument]).unwrap_err();
            assert!(
                error.starts_with(&format!("{argument} is not an option of the isolate")),
                "{error}"
            );
        }
    }

    #[test]
    fn the_effective_role_and_the_conflation() {
        let role = |args: &[&str]| {
            let parsed = options(args);
            (role_name(parsed.role()), parsed.is_conflated())
        };
        let p = ["p", ":7000", "-S", "1"];
        let c = ["c", "127.0.0.1:7000", "-S", "1"];
        let with = |base: &[&'static str], extra: &[&'static str]| [base, extra].concat();
        assert_eq!(role(&p), ("PUBLISHER", false));
        assert_eq!(role(&with(&p, &["-c", "stream"])), ("STREAM_PUBLISHER", false));
        assert_eq!(role(&with(&p, &["-c", "ticker"])), ("PUBLISHER", false));
        assert_eq!(
            role(&with(&p, &["-c", "stream", "--role", "PUBLISHER"])),
            ("PUBLISHER", false)
        );
        assert_eq!(role(&c), ("STREAM_FEED", false));
        assert_eq!(role(&with(&c, &["-c", "ticker"])), ("FEED", true));
        assert_eq!(role(&with(&c, &["--role", "FEED"])), ("FEED", true));
        assert_eq!(
            role(&with(&c, &["--role", "FEED", "--event", "TimeAndSale"])),
            ("FEED", false)
        );
        assert_eq!(
            role(&with(&c, &["-c", "stream", "--role", "FEED", "--event", "TimeAndSale"])),
            ("FEED", false)
        );
        assert_eq!(role(&with(&c, &["--role", "STREAM_FEED"])), ("STREAM_FEED", false));
        // The last -c and --role win.
        assert_eq!(
            role(&with(&c, &["-c", "ticker", "-c", "stream"])),
            ("STREAM_FEED", false)
        );
        assert_eq!(
            role(&with(&c, &["--role", "FEED", "--role", "STREAM_FEED"])),
            ("STREAM_FEED", false)
        );
    }

    #[test]
    fn the_metrics_port_takes_an_optional_host() {
        let metrics = |value: &str| {
            let parsed = options(&["c", "127.0.0.1:7000", "-S", "1", "--metrics-port", value]);
            (parsed.metrics_host, parsed.metrics_port)
        };
        assert_eq!(metrics("0"), ("127.0.0.1".to_owned(), Some(0)));
        assert_eq!(metrics("65535"), ("127.0.0.1".to_owned(), Some(65535)));
        assert_eq!(metrics("0.0.0.0:9100"), ("0.0.0.0".to_owned(), Some(9100)));
        assert_eq!(metrics("[::1]:9100"), ("[::1]".to_owned(), Some(9100)));
        for bad in [":9100", "", "-1", "65536", "123456", "abc", "host:", "host:x", "1e3"] {
            rejected(
                &["c", "127.0.0.1:7000", "-S", "1", "--metrics-port", bad],
                &format!("--metrics-port needs [<host>:]<port>, where <port> is a number from 0 to 65535: '{bad}'"),
            );
        }
    }

    #[test]
    fn a_summary_file_needs_a_writable_directory_at_parse_time() {
        let directory = std::env::temp_dir();
        let file = directory.join(format!("dxfeed-graal-tools-cli-{}.json", std::process::id()));
        let given = file.to_str().unwrap();
        let parsed = options(&["p", ":7000", "-S", "1", "--summary", given]);
        assert_eq!(parsed.summary_file.as_deref(), Some(given));
        // The probe leaves nothing behind, and the summary itself is not created yet.
        assert!(!file.exists());
        let mut probe = file.clone().into_os_string();
        probe.push(".probe");
        assert!(!Path::new(&probe).exists());
        // A directory is not a file, a missing directory and an empty path are errors.
        let directory_name = directory.to_str().unwrap();
        for bad in [directory_name, "no-such-dir-nettest/summary.json", ""] {
            rejected(
                &["p", ":7000", "-S", "1", "--summary", bad],
                &format!("--summary cannot write to the directory of '{bad}'"),
            );
        }
        // A bare file name goes to the current directory; the error of an earlier option wins by position.
        assert!(is_writable_directory_of("nettest-summary-probe-test.json"));
        assert!(!Path::new("nettest-summary-probe-test.json.probe").exists());
        rejected(
            &["p", ":7000", "--summary", "no-such-dir-nettest/summary.json", "-S", "0"],
            "--summary cannot write to the directory of 'no-such-dir-nettest/summary.json'",
        );
    }

    #[test]
    fn invalid_values_are_rejected_with_their_limits() {
        let p = ["p", ":7000", "-S", "10"];
        let with = |extra: &[&'static str]| [&p[..], extra].concat();
        rejected(
            &with(&["--batch", "-5"]),
            "--batch needs a number from 1 to 4194304: '-5'",
        );
        rejected(
            &with(&["--batch", "x"]),
            "--batch needs a number from 1 to 4194304: 'x'",
        );
        rejected(
            &with(&["--batch", "4194305"]),
            "--batch needs a number from 1 to 4194304: '4194305'",
        );
        assert_eq!(options(&with(&["--batch", "4194304"])).batch, 4_194_304);
        // At most 18 digits are read, so leading zeros of a long number do not slip through.
        rejected(
            &with(&["--batch", "0000000000000000001"]),
            "--batch needs a number from 1 to 4194304: '0000000000000000001'",
        );
        assert_eq!(options(&with(&["--batch", "000000000000000001"])).batch, 1);
        rejected(&with(&["-C", "1.5"]), "-C needs a number from 1 to 1000: '1.5'");
        rejected(&with(&["-C", "+1"]), "-C needs a number from 1 to 1000: '+1'");
        rejected(&with(&["--event", "quote"]), "--event is Trade or TimeAndSale: 'quote'");
        rejected(
            &with(&["--role", "feed"]),
            "--role is PUBLISHER or STREAM_PUBLISHER: 'feed'",
        );
        rejected(
            &["p", ":7000", "-S", "10", "-C", "2"],
            "-C 2 cannot be used with the server address ':7000': every instance would bind the same port",
        );
        assert_eq!(options(&["p", "host:7000", "-S", "10", "-C", "2"]).instances, 2);
        assert!(is_server_address(":7000"));
        assert!(!is_server_address("localhost:7000"));
        assert!(!is_server_address(""));
    }

    #[test]
    fn durations() {
        for (text, millis) in [
            ("500ms", Some(500)),
            ("1ms", Some(1)),
            ("1500ms", Some(1_500)),
            ("10s", Some(10_000)),
            ("1m", Some(60_000)),
            ("7m", Some(420_000)),
            ("0ms", None),
            ("0s", None),
            ("0m", None),
            ("10", None),
            ("s", None),
            ("ms", None),
            ("m", None),
            ("-1s", None),
            ("+1s", None),
            ("1.5s", None),
            ("1 s", None),
            ("5h", None),
            ("1H", None),
            ("1S", None),
            // 15 digits are the limit; the product still fits.
            ("999999999999999ms", Some(999_999_999_999_999)),
            ("999999999999999s", Some(999_999_999_999_999_000)),
            ("1000000000000000ms", None),
            ("999999999999999m", None),
            ("99999999999999999999s", None),
        ] {
            assert_eq!(parse_duration(text), millis, "{text}");
        }
        assert_eq!(options(&["p", ":1", "-S", "1", "-s", "500ms"]).period_millis, 500);
        rejected(
            &["p", ":1", "-S", "1", "-s", "0ms"],
            "-s needs a positive time with ms, s or m (for example 10s): '0ms'",
        );
    }

    #[test]
    fn rates() {
        for (text, rate) in [
            ("1", Some(1)),
            ("750", Some(750)),
            ("500k", Some(500_000)),
            ("0.5k", Some(500)),
            ("1M", Some(1_000_000)),
            ("1.5M", Some(1_500_000)),
            ("2.5M", Some(2_500_000)),
            ("1G", Some(1_000_000_000)),
            ("1000G", Some(1_000_000_000_000)),
            ("1.4", Some(1)),
            ("1.5", Some(2)),
            ("0", None),
            ("0.1", None),
            ("0.9", None),
            ("", None),
            ("k", None),
            ("1.2.3", None),
            ("1e3", None),
            ("1e6", None),
            ("+5", None),
            ("-5", None),
            (".5", None),
            ("5.", None),
            ("1K", None),
            ("1m", None),
            ("1g", None),
            ("1 k", None),
            ("1001G", None),
            ("9999999999999G", None),
        ] {
            assert_eq!(parse_rate(text), rate, "{text}");
        }
        for (rate, parsed) in [("*/0.5k", 500), ("*/1.5M", 1_500_000), ("*/1000G", 1_000_000_000_000)] {
            assert_eq!(
                options(&["p", ":1", "-S", "1", "-r", rate]).records_per_second,
                Some(parsed)
            );
        }
        for rate in ["*/1e3", "*/9999999999999G", "*/0"] {
            rejected(
                &["p", ":1", "-S", "1", "-r", rate],
                &format!("-r has a bad rate (a positive number with an optional k, M or G): '{rate}'"),
            );
        }
        rejected(
            &["p", ":1", "-S", "1", "-r", "1M"],
            "-r needs */<rate> (for example */500k): '1M'",
        );
        rejected(
            &["p", ":1", "-S", "1", "-r", "*/1M;5s"],
            "unsupported QD flag: the rate limiter bucket ';<time>' in '-r */1M;5s'",
        );
    }

    #[test]
    fn the_usage_lists_the_supported_options() {
        let usage = usage_text();
        assert!(usage.starts_with("Usage: Tools NetTest p|c <address> -S <symbols> [options]\n\n"));
        assert!(usage.ends_with("\nspec_version nettest/1\n"));
        assert_eq!(usage.matches("spec_version").count(), 1);
        for option in [
            "-S <total>",
            "-C <n>",
            "-r, --rate-limit */<rate>",
            "-s, --stat <period>",
            "-n, --name <name>",
            "-c, --collector <contract>",
            "--event <type>",
            "--role <role>",
            "--batch <n>",
            "--reuse",
            "--duration <time>",
            "--warmup <time>",
            "--summary <file>",
            "--metrics-port [<host>:]<port>",
            "-D<key>=<value>",
            "-Xmx<size>",
            "-XX:<option>",
            "--help",
        ] {
            assert!(usage.contains(option), "the usage lacks {option}");
        }
        assert!(!usage.contains("--legacy"));
        assert!(usage.lines().all(|line| line.len() <= 120), "a long line in the usage");
    }
}
