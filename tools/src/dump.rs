//! The Dump tool: prints and tapes all the events of an address, such as a file, as the Dump tool of the C++ API
//! does.

use std::error::Error;

use dxfeed_graal::{DXEndpoint, Role, system};

use crate::args::{self, Arg, Parsed, optional, required};
use crate::help::Info;
use crate::tape;

pub const INFO: Info = Info {
    name: "Dump",
    short_description: "Dumps all events received from address.",
    description: "\nDumps all events received from address.\n\
                  Enforces a streaming contract for subscription. A wildcard enabled by default.\n\
                  This was designed to receive data from a file.\n\
                  If <types> is not specified, creates a subscription for all available event types.\n\
                  If <symbol> is not specified, the wildcard symbol is used.\n",
    usage: &[
        "Dump <address> [<options>]",
        "Dump <address> <types> [<options>]",
        "Dump <address> <types> <symbols> [<options>]",
    ],
    additional_info: &[],
    args: &[
        required(Arg::Address),
        optional(Arg::Types),
        optional(Arg::Symbols),
        optional(Arg::Properties),
        optional(Arg::Tape),
        optional(Arg::Quiet),
        optional(Arg::Help),
    ],
};

/// Prints and tapes the events until the address has no more, then closes the endpoints.
pub fn run(args: &Parsed) -> Result<(), Box<dyn Error>> {
    let properties = args::parse_properties(args.get(Arg::Properties));
    for (key, value) in &properties {
        system::set_property(key, value)?;
    }
    let types = args::event_types(args.get(Arg::Types))?;
    let symbols = args::symbols(args.get(Arg::Symbols))?;

    let input = DXEndpoint::new_builder()
        .with_role(Role::StreamFeed)
        .with_property(DXEndpoint::DXFEED_WILDCARD_ENABLE_PROPERTY, "true")
        .with_properties(properties.iter().map(|(key, value)| (key, value)))
        .with_name(&format!("{}Tool-Feed", INFO.name))
        .build()?;
    let subscription = input.get_feed().create_subscription_with_types(&types)?;
    if !args.has(Arg::Quiet) {
        tape::print_events(&subscription)?;
    }
    let output = match args.get(Arg::Tape) {
        Some(file) => Some(tape::tape(INFO.name, file, &properties, &subscription)?),
        None => None,
    };
    subscription.add_symbols(symbols)?;

    input.connect(args.get(Arg::Address).unwrap_or_default())?;
    input.await_not_connected()?;
    input.close_and_await_termination()?;
    if let Some(output) = output {
        output.await_processed()?;
        output.close_and_await_termination()?;
    }
    Ok(())
}
