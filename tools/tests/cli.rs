//! The Tools binary: dispatch, usage, version, help screens and the Qds tool; no network needed.

use std::process::{Command, Output};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn tools(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_Tools"))
        .args(args)
        .env("COLUMNS", "100")
        .output()
        .unwrap()
}

/// Runs the tools, checks that they succeed and returns the standard output.
fn stdout(args: &[&str]) -> String {
    let output = tools(args);
    assert!(output.status.success(), "{args:?}: {output:?}");
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn the_usage_lists_the_tools() {
    let usage = stdout(&[]);
    assert!(usage.starts_with(&format!("\nTools {VERSION}\n")), "{usage}");
    assert!(
        usage.contains("\nA collection of useful utilities that use the dxFeed API.\n\nUsage:\n  Tools <tool> [...]\n")
    );
    assert!(
        usage.contains(
            "\nWhere:\n\n  Connect   Connects to specified address(es).\n  Dump      Dumps all events received from address.\n  \
             Help      Help tool.\n  PerfTest  Connects to specified address and calculates performance counters.\n  \
             Qds       A collection of tools ported from the Java qds-tools.\n"
        )
    );
    assert!(usage.contains("\nTo get detailed help on some tool use \"Help <tool>\".\n"));
    assert_eq!(stdout(&["-h"]), usage);
    assert_eq!(stdout(&["--help"]), usage);
    assert_eq!(stdout(&["NoSuchTool"]), usage);
}

#[test]
fn the_version() {
    assert_eq!(stdout(&["--version"]).trim_end(), format!("Tools {VERSION}"));
}

#[test]
fn tool_names_in_any_case() {
    let screen = stdout(&["Qds", "-h"]);
    assert!(
        screen.starts_with(&format!("Tools {VERSION}\n\nQds\n===\n")),
        "{screen}"
    );
    assert_eq!(stdout(&["qds", "--help"]), screen);
    assert_eq!(stdout(&["QDS", "-h"]), screen);
}

#[test]
fn a_parse_error_shows_the_screen_with_the_error() {
    let screen = stdout(&["Qds"]);
    assert!(
        screen.contains("\nQds\n===\n\n  \"qds-args\" argument parsing error. Insufficient parameters.\n\nUsage:\n"),
        "{screen}"
    );
    assert!(screen.contains("  qds-args (pos. 0)  Required. Represents the arguments passed to the qds-tools.\n"));
}

#[test]
fn help_shows_tools_and_articles() {
    assert!(stdout(&["Help", "qds"]).starts_with("\nQds\n===\n"));
    let address = stdout(&["help", "address"]);
    assert!(
        address.starts_with("\nAddress\n=======\n\nTo create a connection"),
        "{address}"
    );
    assert!(stdout(&["Help", "Time", "Format"]).starts_with("\nTime Format\n===========\n"));
    assert_eq!(
        stdout(&["Help", "nothing"]),
        "\n  No help article found for \"nothing\".\n"
    );
    let contents = stdout(&["Help", "contents"]);
    assert!(contents.starts_with("\nHelp articles:\n  Address\n"), "{contents}");
    let all = stdout(&["Help", "all"]);
    assert!(all.starts_with(&format!("{}\n\nAddress\n", "-".repeat(99))), "{all}");
    assert!(all.contains("\nTime Format\n===========\n"));
}

#[test]
fn qds_runs_the_java_tools() {
    let output = tools(&["Qds", "help"]);
    assert!(output.status.success(), "{output:?}");
    let text = String::from_utf8_lossy(&output.stdout) + String::from_utf8_lossy(&output.stderr);
    assert!(
        text.contains("To see list of all articles type \"Help contents\"."),
        "{text}"
    );
    assert!(text.contains("-w|--width <n>"), "{text}");
}
