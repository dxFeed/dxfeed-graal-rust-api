//! The arguments of the tools, their help texts and their parsing, as the C++ tools have them.

use std::collections::HashSet;

use dxfeed_graal::qds::Tools;
use dxfeed_graal::{EventType, Symbol};

/// An argument a tool may take: a positional argument or a named option.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Arg {
    /// The address to connect to, at position 0.
    Address,
    /// The event types, at position 1.
    Types,
    /// The symbols, at position 2.
    Symbols,
    /// The help article, all the arguments joined by spaces.
    Article,
    /// The arguments of the qds-tools, at position 0.
    QdsArgs,
    Properties,
    FromTime,
    Source,
    Tape,
    Quiet,
    ForceStream,
    CpuUsageByCore,
    DetachListener,
    Interval,
    IgnoreExchanges,
    Help,
}

impl Arg {
    /// The name of the argument in the error messages.
    pub fn name(self) -> &'static str {
        match self {
            Arg::Address => "address",
            Arg::Types => "types",
            Arg::Symbols => "symbols",
            Arg::Article => "article",
            Arg::QdsArgs => "qds-args",
            Arg::Properties => "properties",
            Arg::FromTime => "fromTime",
            Arg::Source => "source",
            Arg::Tape => "tape",
            Arg::Quiet => "quiet",
            Arg::ForceStream => "forceStream",
            Arg::CpuUsageByCore => "cpuUsageByCore",
            Arg::DetachListener => "detachListener",
            Arg::Interval => "interval",
            Arg::IgnoreExchanges => "ignoreExchanges",
            Arg::Help => "help",
        }
    }

    /// The position of a positional argument; `None` for an option.
    pub fn position(self) -> Option<usize> {
        match self {
            Arg::Address | Arg::Article | Arg::QdsArgs => Some(0),
            Arg::Types => Some(1),
            Arg::Symbols => Some(2),
            _ => None,
        }
    }

    /// The short and the long name of an option, without dashes; empty when it has none.
    fn names(self) -> (&'static str, &'static str) {
        match self {
            Arg::Properties => ("p", "properties"),
            Arg::FromTime => ("f", "from-time"),
            Arg::Source => ("s", "source"),
            Arg::Tape => ("t", "tape"),
            Arg::Quiet => ("q", "quiet"),
            Arg::ForceStream => ("", "force-stream"),
            Arg::CpuUsageByCore => ("", "cpu-usage-by-core"),
            Arg::DetachListener => ("", "detach-listener"),
            Arg::Interval => ("", "interval"),
            Arg::IgnoreExchanges => ("", "ignore-exchanges"),
            Arg::Help => ("h", "help"),
            _ => ("", ""),
        }
    }

    /// Whether the option takes no value.
    fn is_flag(self) -> bool {
        matches!(
            self,
            Arg::Quiet | Arg::ForceStream | Arg::CpuUsageByCore | Arg::DetachListener | Arg::Help
        )
    }

    /// Whether `token` names the option; `--quite`, the former misspelled name of `--quiet`, still does.
    fn matches(self, token: &str) -> bool {
        let (short, long) = self.names();
        (!short.is_empty() && token.strip_prefix('-') == Some(short))
            || (!long.is_empty() && token.strip_prefix("--") == Some(long))
            || (self == Arg::Quiet && token == "--quite")
    }

    /// The name in the help screen: `address (pos. 0)` or `-p, --properties`.
    pub fn full_name(self) -> String {
        if let Some(position) = self.position() {
            return format!("{} (pos. {position})", self.name());
        }
        match self.names() {
            ("", long) => format!("--{long}"),
            (short, long) => format!("-{short}, --{long}"),
        }
    }

    /// The help text.
    pub fn help_text(self) -> &'static str {
        match self {
            Arg::Address => {
                "The address(es) to connect to retrieve data (see \"Help address\").\n\
                 For Token-Based Authorization, use the following format: \"<address>:<port>[login=entitle:<token>]\"."
            }
            Arg::Types => {
                "Comma-separated list of dxfeed event types (e.g. Quote, TimeAndSale).\n\
                 Use \"all\" for all available event types."
            }
            Arg::Symbols => {
                "Comma-separated list of symbol names to get events for (e.g. \"IBM, AAPL, MSFT\").\n\
                 Use \"all\" for wildcard subscription. The \"dxfeed.wildcard.enable\" property must be set to true to \
                 enable wildcard subscription.\n\
                 A symbol filter set format can also be used as symbols source, for example: \
                 \"ipf[https://demo:demo@tools.dxfeed.com/ipf?TYPE=STOCK]\""
            }
            Arg::Article => "Help article to show.",
            Arg::QdsArgs => "Represents the arguments passed to the qds-tools.",
            Arg::Properties => "Comma-separated list of properties (key-value pair separated by an equals sign).",
            Arg::FromTime => "From-time for the history subscription in standard formats (see \"Help Time Format\").",
            Arg::Source => "Order source for the indexed subscription (e.g. NTV, ntv).",
            Arg::Tape => "Tape all incoming data into the specified file (see \"Help Tape\").",
            Arg::Quiet => "Be quiet, event printing is disabled.",
            Arg::ForceStream => {
                "Enforces a streaming contract for subscription. The StreamFeed role is used instead of Feed."
            }
            Arg::CpuUsageByCore => "Show CPU usage by core (where 1 core = 100%).",
            Arg::DetachListener => "Don't attach a listener. Used for debugging purposes.",
            Arg::Interval => "Measurement interval in seconds.",
            Arg::IgnoreExchanges => {
                "Comma-separated list of exchange codes (e.g. \"Q,Z\") whose events are not measured."
            }
            Arg::Help => "Display this help screen.",
        }
    }
}

/// An argument of a tool and whether it is required.
#[derive(Debug, Clone, Copy)]
pub struct Spec {
    pub arg: Arg,
    pub required: bool,
}

/// A required argument.
pub const fn required(arg: Arg) -> Spec {
    Spec { arg, required: true }
}

/// An optional argument.
pub const fn optional(arg: Arg) -> Spec {
    Spec { arg, required: false }
}

impl Spec {
    /// The help text, after `Required. ` for a required argument.
    pub fn full_help_text(&self) -> String {
        if self.required {
            format!("Required. {}", self.arg.help_text())
        } else {
            self.arg.help_text().to_owned()
        }
    }
}

/// The arguments given to a tool.
#[derive(Debug, Default)]
pub struct Parsed {
    values: Vec<(Arg, String)>,
    flags: HashSet<Arg>,
    /// The tokens that are neither a positional argument nor an option of the tool.
    pub rest: Vec<String>,
}

impl Parsed {
    /// The value of an argument, if given.
    pub fn get(&self, arg: Arg) -> Option<&str> {
        self.values
            .iter()
            .find(|(a, _)| *a == arg)
            .map(|(_, value)| value.as_str())
    }

    /// Whether a flag is given.
    #[cfg_attr(not(test), expect(dead_code, reason = "the event tools of the next changes use it"))]
    pub fn has(&self, arg: Arg) -> bool {
        self.flags.contains(&arg)
    }

    fn is_parsed(&self, arg: Arg) -> bool {
        self.flags.contains(&arg) || self.values.iter().any(|(a, _)| *a == arg)
    }
}

/// The outcome of parsing the arguments of a tool.
#[derive(Debug)]
pub enum ParseResult {
    Ok(Parsed),
    /// The first argument asks for the help screen.
    Help,
    /// The message of a parse error.
    Error(String),
}

/// Parses the arguments of a tool, the tool name excluded, as the C++ tools do.
///
/// `-h` or `--help` as the first argument asks for the help screen. The positional arguments come first, at their
/// positions; an optional one is absent when its token starts with `-` (the C++ tools check only the token after the
/// address, so there `Dump file Quote -q` takes `-q` for the symbols). Each option is taken once, an option with a
/// value only when the value follows; other tokens go to [`Parsed::rest`].
pub fn parse(specs: &[Spec], args: &[String]) -> ParseResult {
    if args.first().is_some_and(|first| Arg::Help.matches(first)) {
        return ParseResult::Help;
    }
    let mut parsed = Parsed::default();
    let mut index = 0;
    for spec in specs {
        let Some(position) = spec.arg.position() else {
            continue;
        };
        if spec.arg == Arg::Article {
            if args.is_empty() {
                return ParseResult::Error(insufficient(spec.arg));
            }
            parsed.values.push((spec.arg, args.join(" ")));
            index = args.len();
        } else if spec.required {
            match args.get(position) {
                Some(value) => parsed.values.push((spec.arg, value.clone())),
                None => return ParseResult::Error(insufficient(spec.arg)),
            }
            index = position + 1;
        } else if index == position
            && let Some(value) = args.get(position).filter(|value| !value.starts_with('-'))
        {
            parsed.values.push((spec.arg, value.clone()));
            index = position + 1;
        }
    }
    while index < args.len() {
        let token = &args[index];
        let spec = specs.iter().find(|spec| {
            spec.arg.position().is_none()
                && spec.arg != Arg::Help
                && !parsed.is_parsed(spec.arg)
                && spec.arg.matches(token)
        });
        match spec {
            Some(spec) if spec.arg.is_flag() => {
                parsed.flags.insert(spec.arg);
                index += 1;
            }
            Some(spec) if index + 1 < args.len() => {
                parsed.values.push((spec.arg, args[index + 1].clone()));
                index += 2;
            }
            _ => {
                parsed.rest.push(token.clone());
                index += 1;
            }
        }
    }
    ParseResult::Ok(parsed)
}

fn insufficient(arg: Arg) -> String {
    if arg == Arg::Symbols {
        "Symbols parsing error. Insufficient parameters.".to_owned()
    } else {
        format!("\"{}\" argument parsing error. Insufficient parameters.", arg.name())
    }
}

/// Parses `key=value` pairs separated by commas; an item without exactly one `=` is skipped.
pub fn parse_properties(properties: Option<&str>) -> Vec<(String, String)> {
    let Some(properties) = properties else {
        return Vec::new();
    };
    properties
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .filter_map(|item| {
            let parts: Vec<&str> = item.split('=').map(str::trim).collect();
            match parts[..] {
                [key, value] => Some((key.to_owned(), value.to_owned())),
                _ => None,
            }
        })
        .collect()
}

/// The upper-case name of an event type in the C++ API, such as `TIME_AND_SALE` for `TimeAndSale`.
#[cfg_attr(not(test), expect(dead_code, reason = "the event tools of the next changes use it"))]
fn upper_case_name(event_type: EventType) -> String {
    let mut name = String::new();
    let mut previous_lower = false;
    for c in event_type.get_name().chars() {
        if c.is_ascii_uppercase() && previous_lower {
            name.push('_');
        }
        previous_lower = c.is_ascii_lowercase() || c.is_ascii_digit();
        name.push(c.to_ascii_uppercase());
    }
    name
}

/// Parses event types: `*` or `all` for every type; otherwise comma-separated names, each either the upper-case name
/// in any case (`QUOTE`, `time_and_sale`) or the class name (`TimeAndSale`). Returns the types and the unknown names.
#[cfg_attr(not(test), expect(dead_code, reason = "the event tools of the next changes use it"))]
pub fn parse_types(types: &str) -> (Vec<EventType>, Vec<String>) {
    let types = types.trim();
    if types.is_empty() {
        return (Vec::new(), Vec::new());
    }
    if matches!(types, "*" | "all" | "All" | "ALL") {
        return (EventType::ALL.to_vec(), Vec::new());
    }
    let mut known = Vec::new();
    let mut unknown = Vec::new();
    for name in types.split(',').map(str::trim).filter(|name| !name.is_empty()) {
        let found = EventType::ALL
            .iter()
            .copied()
            .find(|&t| upper_case_name(t).eq_ignore_ascii_case(name) || t.get_name() == name);
        match found {
            Some(t) if !known.contains(&t) => known.push(t),
            Some(_) => {}
            None => unknown.push(name.to_owned()),
        }
    }
    (known, unknown)
}

/// Parses event types as [`parse_types`] does, `None` meaning all of them, with the C++ error messages.
#[cfg_attr(not(test), expect(dead_code, reason = "the event tools of the next changes use it"))]
pub fn event_types(types: Option<&str>) -> Result<Vec<EventType>, String> {
    let (types, unknown) = parse_types(types.unwrap_or("all"));
    if !unknown.is_empty() {
        let available: Vec<&str> = EventType::ALL.iter().map(|t| t.get_name()).collect();
        return Err(format!(
            "There are unknown event types: {}!\n List of available event types: {}",
            unknown.join(", "),
            available.join(", ")
        ));
    }
    if types.is_empty() {
        return Err("The resulting list of types is empty!".to_owned());
    }
    Ok(types)
}

/// Parses symbols with the symbol list parser of the SDK, trimmed and without repeats, as `IBM, AAPL` lists them;
/// `*` or `all` among them makes the wildcard. An empty list has no symbols, and `None` is the wildcard.
#[cfg_attr(not(test), expect(dead_code, reason = "the event tools of the next changes use it"))]
pub fn symbols(symbols: Option<&str>) -> dxfeed_graal::Result<Vec<Symbol>> {
    let Some(symbols) = symbols.map(str::trim) else {
        return Ok(vec![Symbol::Wildcard]);
    };
    if symbols.is_empty() {
        return Ok(Vec::new());
    }
    let parsed = Tools::parse_symbols(symbols)?;
    let mut seen = HashSet::new();
    let parsed: Vec<&str> = parsed
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty() && seen.insert(*s))
        .collect();
    if parsed.iter().any(|&s| s == "*" || s == "all") {
        return Ok(vec![Symbol::Wildcard]);
    }
    Ok(parsed.into_iter().map(Symbol::from).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONNECT: &[Spec] = &[
        required(Arg::Address),
        required(Arg::Types),
        required(Arg::Symbols),
        optional(Arg::FromTime),
        optional(Arg::Properties),
        optional(Arg::Quiet),
        optional(Arg::Help),
    ];
    const DUMP: &[Spec] = &[
        required(Arg::Address),
        optional(Arg::Types),
        optional(Arg::Symbols),
        optional(Arg::Quiet),
    ];

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    fn ok(specs: &[Spec], args: &[&str]) -> Parsed {
        match parse(specs, &strings(args)) {
            ParseResult::Ok(parsed) => parsed,
            other => panic!("{other:?}"),
        }
    }

    fn error(specs: &[Spec], args: &[&str]) -> String {
        match parse(specs, &strings(args)) {
            ParseResult::Error(e) => e,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn positional_and_named_arguments() {
        let parsed = ok(CONNECT, &["addr", "Quote", "AAPL", "-p", "a=b", "--quiet", "-f", "0"]);
        assert_eq!(parsed.get(Arg::Address), Some("addr"));
        assert_eq!(parsed.get(Arg::Types), Some("Quote"));
        assert_eq!(parsed.get(Arg::Symbols), Some("AAPL"));
        assert_eq!(parsed.get(Arg::Properties), Some("a=b"));
        assert_eq!(parsed.get(Arg::FromTime), Some("0"));
        assert!(parsed.has(Arg::Quiet));
        assert!(parsed.rest.is_empty());
    }

    #[test]
    fn missing_positional_arguments() {
        assert_eq!(
            error(CONNECT, &[]),
            "\"address\" argument parsing error. Insufficient parameters."
        );
        assert_eq!(
            error(CONNECT, &["addr"]),
            "\"types\" argument parsing error. Insufficient parameters."
        );
        assert_eq!(
            error(CONNECT, &["addr", "Quote"]),
            "Symbols parsing error. Insufficient parameters."
        );
    }

    #[test]
    fn help_only_as_the_first_argument() {
        assert!(matches!(parse(CONNECT, &strings(&["-h"])), ParseResult::Help));
        assert!(matches!(parse(CONNECT, &strings(&["--help", "x"])), ParseResult::Help));
        assert_eq!(ok(CONNECT, &["a", "b", "c", "-h"]).rest, ["-h"]);
    }

    #[test]
    fn optional_positional_arguments_stop_at_an_option() {
        let parsed = ok(DUMP, &["file", "-q"]);
        assert_eq!((parsed.get(Arg::Types), parsed.get(Arg::Symbols)), (None, None));
        assert!(parsed.has(Arg::Quiet));
        let parsed = ok(DUMP, &["file", "Quote", "-q"]);
        assert_eq!(
            (parsed.get(Arg::Types), parsed.get(Arg::Symbols)),
            (Some("Quote"), None)
        );
        let parsed = ok(DUMP, &["file", "Quote", "AAPL"]);
        assert_eq!(parsed.get(Arg::Symbols), Some("AAPL"));
    }

    #[test]
    fn an_option_is_taken_once_and_needs_its_value() {
        let parsed = ok(CONNECT, &["a", "b", "c", "-p", "x=1", "--properties", "y=2", "-f"]);
        assert_eq!(parsed.get(Arg::Properties), Some("x=1"));
        assert_eq!(parsed.rest, ["--properties", "y=2", "-f"]);
    }

    #[test]
    fn the_legacy_quiet_name() {
        assert!(ok(CONNECT, &["a", "b", "c", "--quite"]).has(Arg::Quiet));
    }

    #[test]
    fn the_article_joins_all_arguments() {
        let parsed = ok(&[required(Arg::Article)], &["Time", "Format"]);
        assert_eq!(parsed.get(Arg::Article), Some("Time Format"));
        assert_eq!(
            error(&[required(Arg::Article)], &[]),
            "\"article\" argument parsing error. Insufficient parameters."
        );
    }

    #[test]
    fn full_names() {
        assert_eq!(Arg::Symbols.full_name(), "symbols (pos. 2)");
        assert_eq!(Arg::Properties.full_name(), "-p, --properties");
        assert_eq!(Arg::ForceStream.full_name(), "--force-stream");
        assert_eq!(
            required(Arg::QdsArgs).full_help_text(),
            "Required. Represents the arguments passed to the qds-tools."
        );
    }

    #[test]
    fn properties() {
        assert_eq!(
            parse_properties(Some(" a = 1 , b=2,,c,d=e=f, g=")),
            [
                ("a".into(), "1".into()),
                ("b".into(), "2".into()),
                ("g".into(), "".into())
            ]
        );
        assert!(parse_properties(None).is_empty());
    }

    #[test]
    fn types() {
        assert_eq!(parse_types("all").0, EventType::ALL);
        assert_eq!(parse_types("*").0, EventType::ALL);
        assert_eq!(
            parse_types("quote, TRADE_ETH,TimeAndSale,time_and_sale"),
            (
                vec![EventType::Quote, EventType::TradeETH, EventType::TimeAndSale],
                vec![]
            )
        );
        assert_eq!(parse_types("Quote,Nope,timeandsale").1, ["Nope", "timeandsale"]);
        assert_eq!(parse_types(" , "), (vec![], vec![]));
    }

    #[test]
    fn type_errors() {
        let e = event_types(Some("Quote,Nope")).unwrap_err();
        assert!(
            e.starts_with("There are unknown event types: Nope!\n List of available event types: "),
            "{e}"
        );
        assert!(e.contains("TimeAndSale"));
        assert_eq!(
            event_types(Some(",")).unwrap_err(),
            "The resulting list of types is empty!"
        );
        assert_eq!(event_types(None).unwrap(), EventType::ALL);
    }

    #[test]
    fn symbol_lists() {
        assert_eq!(symbols(None).unwrap(), [Symbol::Wildcard]);
        assert_eq!(symbols(Some("all")).unwrap(), [Symbol::Wildcard]);
        assert_eq!(symbols(Some(" * ")).unwrap(), [Symbol::Wildcard]);
        assert!(symbols(Some("  ")).unwrap().is_empty());
        let mut parsed: Vec<String> = symbols(Some("IBM, AAPL,IBM"))
            .unwrap()
            .iter()
            .map(|s| s.to_string())
            .collect();
        parsed.sort();
        assert_eq!(parsed, ["AAPL", "IBM"]);
    }
}
