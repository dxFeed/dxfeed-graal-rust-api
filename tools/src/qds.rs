//! The Qds tool: runs the Java qds-tools.

use dxfeed_graal::qds::Tools;
use dxfeed_graal::system;

use crate::args::{Arg, Parsed, optional, parse_properties, required};
use crate::help::Info;

pub const INFO: Info = Info {
    name: "Qds",
    short_description: "A collection of tools ported from the Java qds-tools.",
    description: "\nA collection of tools ported from the Java qds-tools.\n",
    usage: &["Qds \"<qds-args>\" [<options>]"],
    additional_info: &[],
    args: &[required(Arg::QdsArgs), optional(Arg::Properties), optional(Arg::Help)],
};

/// The arguments of the qds-tools: the first argument and the tokens that are not options, split on spaces.
fn qds_args(args: &Parsed) -> Vec<String> {
    std::iter::once(args.get(Arg::QdsArgs).unwrap_or_default())
        .chain(args.rest.iter().map(String::as_str))
        .flat_map(|arg| arg.split(' '))
        .map(str::trim)
        .filter(|arg| !arg.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Sets the properties and runs the qds-tools, which end the process.
pub fn run(args: &Parsed) -> dxfeed_graal::Result<()> {
    system::set_properties(parse_properties(args.get(Arg::Properties)))?;
    Tools::main(&qds_args(args))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args::{ParseResult, parse};

    #[test]
    fn the_qds_arguments_are_split_and_options_taken_out() {
        let args: Vec<String> = ["connect  demo:7300", "Quote", "-p", "a=b", "AAPL"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let ParseResult::Ok(parsed) = parse(INFO.args, &args) else {
            panic!("not parsed");
        };
        assert_eq!(parsed.get(Arg::Properties), Some("a=b"));
        assert_eq!(qds_args(&parsed), ["connect", "demo:7300", "Quote", "AAPL"]);
    }
}
