//! The help screens and the Help tool, laid out as the C++ tools lay them out.

use std::collections::BTreeSet;

use crate::Tool;
use crate::args::{Arg, Parsed, Spec, optional, required};
use crate::articles::ARTICLES;
use crate::console;

/// The indent of the lines of a screen and the gap between its columns.
const PADDING: usize = 2;

/// What the help screen of a tool shows.
#[derive(Debug)]
pub struct Info {
    pub name: &'static str,
    pub short_description: &'static str,
    /// The description, starting and ending with a line break; may be empty.
    pub description: &'static str,
    pub usage: &'static [&'static str],
    pub additional_info: &'static [&'static str],
    pub args: &'static [Spec],
}

pub const HELP: Info = Info {
    name: "Help",
    short_description: "Help tool.",
    description: "\nDisplays documentation pages.\n",
    usage: &["Help <article>", "Help contents", "Help all"],
    additional_info: &[
        "To see help on some topic type \"Help <topic>\".",
        "To see list of all articles type \"Help contents\".",
        "Use \"Help all\" to generate all existing help articles.",
    ],
    args: &[required(Arg::Article), optional(Arg::Help)],
};

/// The short description of the tools.
const TOOLS_SHORT_DESCRIPTION: &str = "A collection of useful utilities that use the dxFeed API.";

/// The name and version of the tools, such as `Tools 0.1.0`.
pub fn tools_name() -> String {
    format!("Tools {}", env!("CARGO_PKG_VERSION"))
}

/// The header: the name underlined, then the parse error, the description or the short description.
fn header(name: &str, width: usize, text: &str) -> String {
    let length = name.chars().count();
    let fits = length < width;
    let mut result = format!("\n{name}{}", if fits { "\n" } else { "" });
    result.push_str(&"=".repeat(if fits { length } else { width.saturating_sub(1) }));
    result.push('\n');
    result.push_str(text);
    result
}

fn description(info: &Info, error: Option<&str>) -> String {
    match error {
        Some(error) if !error.is_empty() => format!("\n{:PADDING$}{error}\n", ""),
        _ if !info.description.is_empty() => info.description.to_owned(),
        _ if !info.short_description.is_empty() => format!("\n{}\n", info.short_description),
        _ => String::new(),
    }
}

fn usage(usage: &[&str]) -> String {
    let mut result = "Usage:\n".to_owned();
    for line in usage {
        result.push_str(&format!("{:PADDING$}{line}\n", ""));
    }
    result
}

/// The line of an argument: its name in a column `name_field` wide, then its help text wrapped to the width.
fn argument(spec: &Spec, name_field: usize, width: usize) -> String {
    let name = format!(
        "{:PADDING$}{:<field$}{:PADDING$}",
        "",
        spec.arg.full_name(),
        "",
        field = name_field - 2 * PADDING
    );
    let text_width = width.saturating_sub(name_field + 1);
    let mut lines = Vec::new();
    for line in spec.full_help_text().split('\n').map(str::trim) {
        let chars: Vec<char> = line.chars().collect();
        if text_width == 0 || chars.len() <= text_width {
            lines.push(line.to_owned());
        } else {
            lines.extend(chars.chunks(text_width).map(|chunk| chunk.iter().collect::<String>()));
        }
    }
    let mut result = String::new();
    for (i, line) in lines.iter().enumerate() {
        if i == 0 {
            result.push_str(&format!("{name}{line}\n"));
        } else if line.is_empty() {
            result.push('\n');
        } else {
            result.push_str(&format!("{:name_field$}{line}\n", ""));
        }
    }
    result
}

fn additional_info(lines: &[&str]) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let mut result = "\n".to_owned();
    for line in lines {
        result.push_str(line);
        result.push('\n');
    }
    result
}

/// The help screen of a tool, with the parse error in place of the description if there is one.
pub fn tool_screen(info: &Info, error: Option<&str>) -> String {
    let width = console::width();
    let longest = info
        .args
        .iter()
        .map(|spec| spec.arg.full_name().chars().count())
        .max()
        .unwrap_or(0);
    let name_field = PADDING + longest + PADDING;
    let mut result = header(info.name, width, &description(info, error));
    result.push('\n');
    result.push_str(&usage(info.usage));
    result.push_str("\nWhere:\n\n");
    for spec in info.args {
        result.push_str(&argument(spec, name_field, width));
    }
    result.push_str(&additional_info(info.additional_info));
    if let Some((_, content)) = article(info.name) {
        result.push_str(&format!("\n{content}\n"));
    }
    result
}

/// The screen of the tools: the usage and the list of the tools.
pub fn tools_screen() -> String {
    let width = console::width();
    let longest = Tool::ALL.iter().map(|tool| tool.info().name.len()).max().unwrap_or(0);
    let name_field = PADDING + longest + PADDING;
    let mut result = header(&tools_name(), width, &format!("\n{TOOLS_SHORT_DESCRIPTION}\n"));
    result.push('\n');
    result.push_str(&usage(&["Tools <tool> [...]"]));
    result.push_str("\nWhere:\n\n");
    let mut tools: Vec<&Info> = Tool::ALL.iter().map(|tool| tool.info()).collect();
    tools.sort_by_key(|info| info.name);
    for info in tools {
        result.push_str(&format!(
            "{:PADDING$}{:<field$}{:PADDING$}{}\n",
            "",
            info.name,
            "",
            info.short_description,
            field = name_field - 2 * PADDING
        ));
    }
    result.push_str(&additional_info(&[
        "To get detailed help on some tool use \"Help <tool>\".",
    ]));
    result
}

/// The embedded article with this name, in any case.
fn article(name: &str) -> Option<(&'static str, &'static str)> {
    ARTICLES.iter().copied().find(|(key, _)| key.eq_ignore_ascii_case(name))
}

/// The screen of a tool or an article, in any case; `None` if there is none.
pub fn screen(name: &str) -> Option<String> {
    if let Some(tool) = Tool::find(name) {
        return Some(tool_screen(tool.info(), None));
    }
    let (name, content) = article(name)?;
    let mut result = header(name, console::width(), "");
    result.push('\n');
    result.push_str(content);
    result.push('\n');
    Some(result)
}

/// The names of all the tools and articles, sorted.
fn all_names() -> BTreeSet<&'static str> {
    Tool::ALL
        .iter()
        .map(|tool| tool.info().name)
        .chain(ARTICLES.iter().map(|(name, _)| *name))
        .collect()
}

/// The Help tool: the screen of a tool or an article, the list of them (`contents`) or all of them (`all`).
pub fn run(args: &Parsed) {
    console::print(&help(args.get(Arg::Article).unwrap_or_default()));
}

fn help(article: &str) -> String {
    let separator = format!("{}\n", "-".repeat(console::width().saturating_sub(1)));
    if article.eq_ignore_ascii_case("all") {
        let mut result = separator.clone();
        for name in all_names() {
            result.push_str(&screen(name).unwrap_or_default());
            result.push('\n');
            result.push_str(&separator);
        }
        result
    } else if article.eq_ignore_ascii_case("contents") {
        let mut result = "\nHelp articles:\n".to_owned();
        for name in all_names() {
            result.push_str(&format!("{:PADDING$}{name}\n", ""));
        }
        result
    } else {
        match screen(article) {
            Some(screen) => format!("{screen}\n"),
            None => format!("\n{:PADDING$}No help article found for \"{article}\".\n", ""),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_argument_in_its_column() {
        let spec = optional(Arg::Quiet);
        assert_eq!(
            argument(&spec, 15, 80),
            "  -q, --quiet  Be quiet, event printing is disabled.\n"
        );
    }

    #[test]
    fn a_long_help_text_wraps() {
        let spec = optional(Arg::Quiet);
        let pad = " ".repeat(17);
        assert_eq!(
            argument(&spec, 17, 30),
            format!("  -q, --quiet    Be quiet, ev\n{pad}ent printing\n{pad} is disabled\n{pad}.\n")
        );
    }

    #[test]
    fn required_arguments_and_line_breaks() {
        let text = argument(&required(Arg::Types), 20, 200);
        assert_eq!(
            text,
            "  types (pos. 1)    Required. Comma-separated list of dxfeed event types (e.g. Quote, TimeAndSale).\n\
             \x20                   Use \"all\" for all available event types.\n"
        );
    }

    #[test]
    fn the_help_screen_of_a_tool() {
        let screen = tool_screen(&HELP, None);
        assert!(screen.starts_with("\nHelp\n====\n\nDisplays documentation pages.\n\nUsage:\n  Help <article>\n"));
        assert!(screen.contains("\nWhere:\n\n  article (pos. 0)  Required. Help article to show.\n"));
        assert!(screen.contains("  -h, --help        Display this help screen.\n"));
        assert!(screen.ends_with("\nUse \"Help all\" to generate all existing help articles.\n"));
    }

    #[test]
    fn a_parse_error_replaces_the_description() {
        let screen = tool_screen(&HELP, Some("oops"));
        assert!(screen.starts_with("\nHelp\n====\n\n  oops\n\nUsage:\n"));
    }

    #[test]
    fn articles_in_any_case() {
        assert!(
            screen("time format")
                .unwrap()
                .starts_with("\nTime Format\n===========\n\n")
        );
        assert!(screen("nothing").is_none());
        assert_eq!(help("nothing"), "\n  No help article found for \"nothing\".\n");
        let contents = help("contents");
        assert!(contents.starts_with("\nHelp articles:\n  Address\n  ClientSocket\n"));
        assert!(contents.contains("  Help\n") && contents.contains("  Qds\n") && contents.contains("  Time Format\n"));
    }
}
