//! Tools: a collection of useful utilities that use the dxFeed API, as the Tools of the C++ and .NET APIs.
//!
//! `Tools <tool> [...]` runs a tool; `Tools Help <tool>` shows its arguments.

mod args;
mod articles;
mod connect;
mod console;
mod dump;
mod help;
mod qds;
mod tape;

use std::error::Error;
use std::process::ExitCode;

use dxfeed_graal::system;

use crate::args::ParseResult;
use crate::help::Info;

/// A tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Connect,
    Dump,
    Help,
    Qds,
}

impl Tool {
    /// Every tool.
    pub const ALL: &'static [Tool] = &[Tool::Connect, Tool::Dump, Tool::Help, Tool::Qds];

    /// The tool with this name, in any case.
    pub fn find(name: &str) -> Option<Tool> {
        Tool::ALL
            .iter()
            .copied()
            .find(|tool| tool.info().name.eq_ignore_ascii_case(name))
    }

    /// The help screen contents of the tool.
    pub fn info(self) -> &'static Info {
        match self {
            Tool::Connect => &connect::INFO,
            Tool::Dump => &dump::INFO,
            Tool::Help => &help::HELP,
            Tool::Qds => &qds::INFO,
        }
    }

    fn run(self, args: &args::Parsed) -> Result<(), Box<dyn Error>> {
        match self {
            Tool::Connect => connect::run(args)?,
            Tool::Dump => dump::run(args)?,
            Tool::Help => help::run(args),
            Tool::Qds => qds::run(args)?,
        }
        Ok(())
    }
}

fn run(args: &[String]) -> Result<(), Box<dyn Error>> {
    system::set_property("dxfeed.wildcard.enable", "true")?;
    // Enables the experimental dxLink connector and its scheme.
    system::set_property("dxfeed.experimental.dxlink.enable", "true")?;
    system::set_property("scheme", "ext:opt:sysprops,resource:dxlink.xml")?;

    let Some(first) = args.first().filter(|first| !matches!(first.as_str(), "-h" | "--help")) else {
        console::println(&help::tools_screen());
        return Ok(());
    };
    if first == "--version" {
        console::println(&help::tools_name());
        return Ok(());
    }
    let Some(tool) = Tool::find(first) else {
        console::println(&help::tools_screen());
        return Ok(());
    };
    match args::parse(tool.info().args, &args[1..]) {
        ParseResult::Help => {
            console::println(&help::tools_name());
            console::println(&help::tool_screen(tool.info(), None));
        }
        ParseResult::Error(error) => {
            console::println(&help::tools_name());
            console::println(&help::tool_screen(tool.info(), Some(&error)));
        }
        ParseResult::Ok(parsed) => tool.run(&parsed)?,
    }
    Ok(())
}

fn main() -> ExitCode {
    // An argument that is not UTF-8 is taken with its invalid bytes replaced, as the C++ tools take any bytes.
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
