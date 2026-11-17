//! The command line of NetTest: the options, their parsing and validation and the usage text.

use dxfeed_graal::Role;

/// The largest symbol pool, batch and number of instances that the tool accepts. A batch shares one time, so it holds
/// at most 2^22 events: the 22-bit sequence must not wrap inside one batch.
const MAX_SYMBOLS: i64 = 100_000_000;
const MAX_BATCH: i64 = 1 << 22;
const MAX_INSTANCES: i64 = 1_000;

/// The synopsis of the tool, the usage line of its help screen.
pub const SYNOPSIS: &str = "NetTest p <address> -S <symbols> [options]";

/// The lines of the usage text after the synopsis.
pub const OPTION_LINES: &[&str] = &[
    "  <address>            host:port to connect to, :port to listen on",
    "  -S <total>           the symbol pool: <total> random symbols of 1-6 capital letters",
    "  -S <total>@<file>    the first <total> symbols of the IPF file, in file order",
    "  -C <n>               the number of instances, default 1 (not above 1 with a :port address)",
    "  -r */<rate>          the total rate in records per second, with k, M or G (for example */1M)",
    "  -s <period>          the statistics period, default 10s (ms, s or m)",
    "  -n <name>            the name in the statistics line, default NetTest",
    "  --batch <n>          the events per publishing call, default 100",
    "  --event <type>       Trade (default) or TimeAndSale",
    "  --reuse              republish the pre-built events in place instead of copying them",
    "  --role <role>        the role of the endpoints: PUBLISHER (default) or STREAM_PUBLISHER",
    "  --duration <time>    end the run after the time (ms, s or m); Ctrl+C ends it at once",
    "  -D<key>=<value>      set a system property before the first endpoint is created",
    "  -Xmx<size>, -Xms<size>, -Xmn<size>, -Xss<size>",
    "                       the maximum, initial and young generation heap sizes and the thread stack size of the",
    "                       isolate (512m, 1g)",
    "  -XX:<option>         an option of the isolate (for example -XX:+PrintGC -XX:+PrintGCSummary)",
    "",
    "The consumer mode 'c' and the QD flags -c, /r and /s symbol splits, {len,max} symbol lengths and the",
    "rate limiter bucket ;<time> are not supported.",
];

/// The usage text, ending with a line break.
pub fn usage_text() -> String {
    let mut text = format!("Usage: Tools {SYNOPSIS}\n\n");
    for line in OPTION_LINES {
        text.push_str(line);
        text.push('\n');
    }
    text
}

/// The event type that the instances publish.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    Trade,
    TimeAndSale,
}

/// The parsed and validated command line of the publisher mode.
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
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
    /// `--reuse`.
    pub reuse: bool,
    /// `--role`.
    pub role: Role,
    /// `--duration` in milliseconds; `None` when only Ctrl+C ends the run.
    pub duration_millis: Option<i64>,
    /// `-D<key>=<value>` in command-line order.
    pub properties: Vec<(String, String)>,
    /// `-Xmx`, `-Xms`, `-Xmn`, `-Xss` and `-XX:` as they are, in command-line order.
    pub isolate_arguments: Vec<String>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            address: String::new(),
            total_symbols: 0,
            symbols_file: None,
            instances: 1,
            records_per_second: None,
            period_millis: 10_000,
            name: "NetTest".to_owned(),
            batch: 100,
            event: EventKind::Trade,
            reuse: false,
            role: Role::Publisher,
            duration_millis: None,
            properties: Vec::new(),
            isolate_arguments: Vec::new(),
        }
    }
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
/// non-positive or an overflowing value.
pub fn parse_duration(text: &str) -> Option<i64> {
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
pub fn parse_rate(text: &str) -> Option<i64> {
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
    if rate < 1.0 || rate > 1e12 {
        return None;
    }
    Some(rate.round() as i64)
}

/// Whether the address makes an endpoint listen (it starts with `:`).
pub fn is_server_address(address: &str) -> bool {
    address.starts_with(':')
}

/// The state of one parsing.
#[derive(Default)]
struct Parser {
    options: Options,
    address_seen: bool,
    symbols_seen: bool,
}

impl Parser {
    fn parse_symbols(&mut self, value: &str) -> Result<(), String> {
        let at = value.find('@');
        let head = &value[..at.unwrap_or(value.len())];
        if head.contains('{') {
            return Err(format!(
                "unsupported QD flag: the symbol length '{{len,max}}' in '-S {value}'"
            ));
        }
        if head.contains('/') {
            return Err(format!(
                "unsupported QD flag: the symbol split '/r' or '/s' in '-S {value}'"
            ));
        }
        let Some(total) = parse_positive(head, MAX_SYMBOLS) else {
            return Err(format!(
                "-S needs <total> or <total>@<file>, where <total> is a number from 1 to {MAX_SYMBOLS}: '{value}'"
            ));
        };
        self.options.total_symbols = total;
        self.options.symbols_file = None;
        if let Some(at) = at {
            let file = &value[at + 1..];
            if file.is_empty() {
                return Err(format!("-S has no file name after '@': '{value}'"));
            }
            self.options.symbols_file = Some(file.to_owned());
        }
        self.symbols_seen = true;
        Ok(())
    }

    fn parse_rate_option(&mut self, value: &str) -> Result<(), String> {
        if value.contains(';') {
            return Err(format!(
                "unsupported QD flag: the rate limiter bucket ';<time>' in '-r {value}'"
            ));
        }
        if value.len() < 3 || !value.starts_with("*/") {
            return Err(format!("-r needs */<rate> (for example */500k): '{value}'"));
        }
        let Some(rate) = parse_rate(&value[2..]) else {
            return Err(format!(
                "-r has a bad rate (a positive number with an optional k, M or G): '{value}'"
            ));
        };
        self.options.records_per_second = Some(rate);
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

    fn run(&mut self, args: &[String]) -> Result<(), String> {
        let Some(mode) = args.first() else {
            return Err("no mode".to_owned());
        };
        if mode == "c" {
            return Err("unsupported mode 'c' (the consumer mode of QD NetTest); only the mode 'p' exists".to_owned());
        }
        if mode != "p" {
            return Err(format!("unknown mode '{mode}'; only the mode 'p' exists"));
        }
        let mut rest = args[1..].iter();
        while let Some(argument) = rest.next() {
            // The flags with a value take the next argument as it is.
            let mut value = |flag: &str| rest.next().ok_or_else(|| format!("missing value of {flag}"));
            match argument.as_str() {
                "-S" => {
                    let text = value("-S")?;
                    self.parse_symbols(text)?;
                }
                "-C" => self.options.instances = parse_number("-C", value("-C")?, MAX_INSTANCES)? as usize,
                "-r" => {
                    let text = value("-r")?;
                    self.parse_rate_option(text)?;
                }
                "-s" => self.options.period_millis = parse_duration_option("-s", value("-s")?)?,
                "-n" => {
                    let text = value("-n")?;
                    if text.is_empty() {
                        return Err("-n needs a non-empty name".to_owned());
                    }
                    self.options.name = text.clone();
                }
                "--batch" => self.options.batch = parse_number("--batch", value("--batch")?, MAX_BATCH)? as usize,
                "--event" => {
                    self.options.event = match value("--event")?.as_str() {
                        "Trade" => EventKind::Trade,
                        "TimeAndSale" => EventKind::TimeAndSale,
                        other => return Err(format!("--event is Trade or TimeAndSale: '{other}'")),
                    }
                }
                "--role" => {
                    self.options.role = match value("--role")?.as_str() {
                        "PUBLISHER" => Role::Publisher,
                        "STREAM_PUBLISHER" => Role::StreamPublisher,
                        other => return Err(format!("--role is PUBLISHER or STREAM_PUBLISHER: '{other}'")),
                    }
                }
                "--duration" => {
                    self.options.duration_millis = Some(parse_duration_option("--duration", value("--duration")?)?)
                }
                "--reuse" => self.options.reuse = true,
                "--legacy" => {
                    return Err(
                        "--legacy is an option of the C tool only (the deprecated C publishing function)".to_owned(),
                    );
                }
                "-c" => return Err("unsupported QD flag: -c (the collector)".to_owned()),
                _ if argument.starts_with("-D") => self.parse_property(argument)?,
                _ if argument.starts_with("-X") => self.parse_isolate_argument(argument)?,
                _ if argument.len() > 1 && argument.starts_with('-') => {
                    return Err(format!("unknown flag {argument}"));
                }
                _ if !self.address_seen => {
                    self.options.address = argument.clone();
                    self.address_seen = true;
                }
                _ => {
                    return Err(format!(
                        "unexpected argument '{argument}' after the address '{}'",
                        self.options.address
                    ));
                }
            }
        }
        if !self.address_seen || self.options.address.is_empty() {
            return Err("the address is missing".to_owned());
        }
        if !self.symbols_seen {
            return Err("-S is missing".to_owned());
        }
        if self.options.instances > 1 && is_server_address(&self.options.address) {
            return Err(format!(
                "-C {} cannot be used with the server address '{}': every instance would bind the same port",
                self.options.instances, self.options.address
            ));
        }
        Ok(())
    }
}

fn parse_number(flag: &str, value: &str, limit: i64) -> Result<i64, String> {
    parse_positive(value, limit).ok_or_else(|| format!("{flag} needs a number from 1 to {limit}: '{value}'"))
}

fn parse_duration_option(flag: &str, value: &str) -> Result<i64, String> {
    parse_duration(value)
        .ok_or_else(|| format!("{flag} needs a positive time with ms, s or m (for example 10s): '{value}'"))
}

/// Parses the arguments after `NetTest`, the mode first; the error is the message of the first failure.
pub fn parse(args: &[String]) -> Result<Options, String> {
    let mut parser = Parser::default();
    parser.run(args)?;
    Ok(parser.options)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_args(args: &[&str]) -> Result<Options, String> {
        parse(&args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>())
    }

    fn options(args: &[&str]) -> Options {
        parse_args(args).unwrap_or_else(|error| panic!("{args:?}: {error}"))
    }

    #[track_caller]
    fn rejected(args: &[&str], part: &str) {
        match parse_args(args) {
            Ok(options) => panic!("{args:?} accepted: {options:?}"),
            Err(error) => assert!(error.contains(part), "{args:?}: '{error}' lacks '{part}'"),
        }
    }

    #[test]
    fn the_defaults_of_a_minimal_command_line() {
        let parsed = options(&["p", ":7000", "-S", "10"]);
        assert_eq!(
            parsed,
            Options {
                address: ":7000".to_owned(),
                total_symbols: 10,
                ..Options::default()
            }
        );
        assert_eq!(parsed.period_millis, 10_000);
        assert_eq!(parsed.name, "NetTest");
        assert_eq!(parsed.batch, 100);
        assert_eq!(parsed.event, EventKind::Trade);
        assert_eq!(parsed.instances, 1);
        assert_eq!(parsed.records_per_second, None);
        assert_eq!(parsed.duration_millis, None);
        assert_eq!(parsed.role, Role::Publisher);
        assert!(!parsed.reuse);
    }

    #[test]
    fn every_option_lands_in_the_options() {
        let parsed = options(&[
            "p",
            "host:7000",
            "-S",
            "5",
            "-S",
            "20@symbols.ipf",
            "-C",
            "3",
            "-r",
            "*/1k",
            "-r",
            "*/2M",
            "-s",
            "1500ms",
            "-n",
            "run1",
            "--batch",
            "50",
            "--event",
            "TimeAndSale",
            "--reuse",
            "--role",
            "STREAM_PUBLISHER",
            "--duration",
            "2m",
            "-Ddxscheme.nanoTime=true",
            "-Dk=a=b",
            "-Dempty=",
            "-Xmx64m",
            "-XX:+PrintGC",
            "-Xmn2m",
            "-Xss1m",
        ]);
        assert_eq!(
            parsed,
            Options {
                address: "host:7000".to_owned(),
                total_symbols: 20,
                symbols_file: Some("symbols.ipf".to_owned()),
                instances: 3,
                records_per_second: Some(2_000_000),
                period_millis: 1_500,
                name: "run1".to_owned(),
                batch: 50,
                event: EventKind::TimeAndSale,
                reuse: true,
                role: Role::StreamPublisher,
                duration_millis: Some(120_000),
                properties: vec![
                    ("dxscheme.nanoTime".to_owned(), "true".to_owned()),
                    ("k".to_owned(), "a=b".to_owned()),
                    ("empty".to_owned(), String::new()),
                ],
                isolate_arguments: vec![
                    "-Xmx64m".to_owned(),
                    "-XX:+PrintGC".to_owned(),
                    "-Xmn2m".to_owned(),
                    "-Xss1m".to_owned(),
                ],
            }
        );
        // A later -S without a file drops the file of an earlier one.
        assert_eq!(options(&["p", ":1", "-S", "5@a.ipf", "-S", "7"]).symbols_file, None);
        // Flags may come before the address.
        assert_eq!(options(&["p", "-S", "10", ":7000"]).address, ":7000");
    }

    #[test]
    fn symbols_file_forms() {
        assert_eq!(
            options(&["p", ":1", "-S", "10@symbols.ipf"]).symbols_file.as_deref(),
            Some("symbols.ipf")
        );
        assert_eq!(
            options(&["p", ":1", "-S", "10@C:/data/some/symbols.ipf"])
                .symbols_file
                .as_deref(),
            Some("C:/data/some/symbols.ipf")
        );
        assert_eq!(
            options(&["p", ":1", "-S", "10@C:\\data\\symbols.ipf"])
                .symbols_file
                .as_deref(),
            Some("C:\\data\\symbols.ipf")
        );
        assert_eq!(
            options(&["p", ":1", "-S", "10@../dir/s.ipf"]).symbols_file.as_deref(),
            Some("../dir/s.ipf")
        );
    }

    #[test]
    fn qd_flags_and_legacy_are_rejected_by_name() {
        rejected(&["c", "127.0.0.1:7000", "-S", "10"], "mode 'c'");
        rejected(&["p", ":7000", "-S", "10", "-c", "stream"], "unsupported QD flag: -c");
        rejected(&["p", ":7000", "-S", "10/r5"], "/r");
        rejected(&["p", ":7000", "-S", "10/s5"], "/s");
        rejected(&["p", ":7000", "-S", "10{3}"], "{len,max}");
        rejected(&["p", ":7000", "-S", "10{3,5}"], "{len,max}");
        rejected(&["p", ":7000", "-S", "10", "-r", "*/1M;5s"], ";<time>");
        rejected(&["x", ":7000", "-S", "10"], "unknown mode");
        rejected(
            &["p", ":7000", "-S", "10", "--legacy"],
            "--legacy is an option of the C tool only",
        );
    }

    #[test]
    fn more_than_one_instance_needs_a_connecting_address() {
        assert_eq!(
            parse_args(&["p", ":7000", "-S", "10", "-C", "2"]),
            Err(
                "-C 2 cannot be used with the server address ':7000': every instance would bind the same port"
                    .to_owned()
            )
        );
        assert_eq!(options(&["p", "host:7000", "-S", "10", "-C", "2"]).instances, 2);
        assert_eq!(options(&["p", ":7000", "-S", "10", "-C", "1"]).instances, 1);
        assert!(is_server_address(":7000"));
        assert!(!is_server_address("localhost:7000"));
        assert!(!is_server_address(""));
    }

    #[test]
    fn invalid_command_lines_are_rejected() {
        rejected(&[], "no mode");
        rejected(&["p", "-S", "10"], "the address is missing");
        rejected(&["p", ":7000"], "-S is missing");
        rejected(&["p", ":7000", "-S", "10", "--batch", "0"], "--batch");
        rejected(&["p", ":7000", "-S", "10", "--batch", "-5"], "--batch");
        rejected(&["p", ":7000", "-S", "10", "--batch", "x"], "--batch");
        rejected(
            &["p", ":7000", "-S", "10", "--batch", "4194305"],
            "--batch needs a number from 1 to 4194304",
        );
        assert_eq!(
            options(&["p", ":7000", "-S", "10", "--batch", "4194304"]).batch,
            4_194_304
        );
        rejected(&["p", ":7000", "-S", "10", "-r", "*/0"], "-r has a bad rate");
        rejected(&["p", ":7000", "-S", "10", "-r", "*/-1M"], "-r");
        rejected(&["p", ":7000", "-S", "10", "-r", "*/abc"], "-r");
        rejected(
            &["p", ":7000", "-S", "10", "-r", "1M"],
            "-r needs */<rate> (for example */500k): '1M'",
        );
        rejected(&["p", ":7000", "-S", "10", "-r", "*/0.1"], "-r has a bad rate");
        rejected(&["p", ":7000", "-S", "10", "-r", "*/1e3"], "-r has a bad rate");
        rejected(&["p", ":7000", "-S", "10", "-s", "0s"], "-s");
        rejected(
            &["p", ":7000", "-S", "10", "-s", "10"],
            "-s needs a positive time with ms, s or m (for example 10s): '10'",
        );
        rejected(&["p", ":7000", "-S", "10", "-s", "5h"], "-s");
        rejected(&["p", ":7000", "-S", "10", "--duration", "0s"], "--duration");
        rejected(&["p", ":7000", "-S", "10", "--duration", "5h"], "--duration");
        rejected(
            &["p", ":7000", "-S", "10", "--event", "Quote"],
            "--event is Trade or TimeAndSale: 'Quote'",
        );
        rejected(
            &["p", ":7000", "-S", "10", "-C", "0"],
            "-C needs a number from 1 to 1000: '0'",
        );
        rejected(
            &["p", ":7000", "-S", "10", "-C", "1001"],
            "-C needs a number from 1 to 1000",
        );
        rejected(
            &["p", ":7000", "-S", "0"],
            "-S needs <total> or <total>@<file>, where <total> is a number from 1 to 100000000: '0'",
        );
        rejected(&["p", ":7000", "-S", "abc"], "-S needs");
        rejected(&["p", ":7000", "-S", "10@"], "-S has no file name after '@': '10@'");
        rejected(&["p", ":7000", "-S", "10", "-n", ""], "-n needs a non-empty name");
        rejected(&["p", ":7000", "-S", "10", "--unknown"], "unknown flag --unknown");
        rejected(&["p", ":7000", "-S", "10", "-h"], "unknown flag -h");
        rejected(
            &["p", ":7000", "-S", "10", "-Dnovalue"],
            "-D needs <key>=<value>: '-Dnovalue'",
        );
        rejected(&["p", ":7000", "-S", "10", "-D=x"], "-D needs <key>=<value>: '-D=x'");
        for argument in [
            "-X", "-XX", "-XX:", "-Xmx", "-Xss", "-Xfoo", "-Xsz64k", "-Xnm2m", "-XXfoo",
        ] {
            rejected(
                &["p", ":7000", "-S", "10", argument],
                &format!(
                    "{argument} is not an option of the isolate, which takes -Xmx<size>, -Xms<size>, -Xmn<size>, \
                     -Xss<size> and -XX:<option>"
                ),
            );
        }
        rejected(
            &["p", ":7000", "extra", "-S", "10"],
            "unexpected argument 'extra' after the address ':7000'",
        );
        rejected(
            &["p", ":7000", "-S", "10", "--role", "FEED"],
            "--role is PUBLISHER or STREAM_PUBLISHER: 'FEED'",
        );
    }

    #[test]
    fn the_first_error_wins() {
        rejected(&["p", ":7000", "--unknown", "-S", "0"], "unknown flag --unknown");
    }

    #[test]
    fn a_flag_with_a_missing_value_is_rejected() {
        for flag in [
            "-S",
            "-C",
            "-r",
            "-s",
            "-n",
            "--batch",
            "--event",
            "--duration",
            "--role",
        ] {
            rejected(&["p", ":7000", "-S", "10", flag], &format!("missing value of {flag}"));
        }
    }

    #[test]
    fn durations_and_rates() {
        assert_eq!(parse_duration("500ms"), Some(500));
        assert_eq!(parse_duration("1500ms"), Some(1_500));
        assert_eq!(parse_duration("10s"), Some(10_000));
        assert_eq!(parse_duration("2m"), Some(120_000));
        for bad in ["10", "s", "ms", "0ms", "-1s", "1.5s", "99999999999999999999s", "5h"] {
            assert_eq!(parse_duration(bad), None, "{bad}");
        }
        assert_eq!(parse_rate("1"), Some(1));
        assert_eq!(parse_rate("500k"), Some(500_000));
        assert_eq!(parse_rate("0.5k"), Some(500));
        assert_eq!(parse_rate("1M"), Some(1_000_000));
        assert_eq!(parse_rate("2M"), Some(2_000_000));
        assert_eq!(parse_rate("2.5M"), Some(2_500_000));
        assert_eq!(parse_rate("1G"), Some(1_000_000_000));
        assert_eq!(parse_rate("750"), Some(750));
        for bad in ["0", "0.1", "", "k", "1.2.3", "1e3", "+5", ".5", "5.", "9999999999999G"] {
            assert_eq!(parse_rate(bad), None, "{bad}");
        }
        assert_eq!(
            options(&["p", ":1", "-S", "1", "-r", "*/0.5k"]).records_per_second,
            Some(500)
        );
    }

    #[test]
    fn roles() {
        assert_eq!(
            options(&["p", ":1", "-S", "1", "--role", "PUBLISHER"]).role,
            Role::Publisher
        );
        assert_eq!(
            options(&["p", ":1", "-S", "1", "--role", "STREAM_PUBLISHER"]).role,
            Role::StreamPublisher
        );
    }

    #[test]
    fn the_usage_text() {
        let usage = usage_text();
        assert!(usage.starts_with("Usage: Tools NetTest p <address> -S <symbols> [options]\n\n"));
        assert!(usage.contains("\n  --role <role>        the role of the endpoints: PUBLISHER (default) or"));
        assert!(!usage.contains("--legacy"));
        assert!(usage.ends_with("rate limiter bucket ;<time> are not supported.\n"));
    }
}
