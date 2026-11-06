//! The QDS tools of the SDK: symbol list parsing and the tools runner without arguments; no network needed.

use dxfeed_graal::Error;
use dxfeed_graal::qds::Tools;

fn sorted(mut symbols: Vec<String>) -> Vec<String> {
    symbols.sort();
    symbols
}

#[test]
fn parse_symbols_splits_a_list() {
    let symbols = sorted(Tools::parse_symbols("AAPL,IBM").unwrap());
    assert_eq!(symbols, ["AAPL", "IBM"]);
}

#[test]
fn parse_symbols_keeps_the_wildcard() {
    assert_eq!(Tools::parse_symbols("*").unwrap(), ["*"]);
}

#[test]
fn parse_symbols_rejects_an_empty_list() {
    assert!(matches!(Tools::parse_symbols(""), Err(Error::Java(_))));
}

#[test]
fn parse_symbols_rejects_an_invalid_filter() {
    assert!(matches!(Tools::parse_symbols("AAPL,("), Err(Error::Java(_))));
}

#[test]
fn parse_symbols_rejects_a_nul_byte() {
    assert!(matches!(Tools::parse_symbols("AAPL\0"), Err(Error::InvalidArgument(_))));
}

#[test]
fn main_without_arguments_returns() {
    Tools::main::<&str>(&[]).unwrap();
}

#[test]
fn main_rejects_a_nul_byte() {
    assert!(matches!(Tools::main(&["help\0"]), Err(Error::InvalidArgument(_))));
}
