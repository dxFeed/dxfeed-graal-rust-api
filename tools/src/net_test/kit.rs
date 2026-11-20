//! The kit of the NetTest specification: the vectors that the SDK archive ships next to the specification. The build
//! script of the tools hands over its directory.

use std::path::PathBuf;

use serde_json::Value;

/// The path of a kit file.
fn kit_path(name: &str) -> PathBuf {
    PathBuf::from(env!("NETTEST_KIT_DIR")).join(name)
}

/// Reads a kit file as text; the message of a failure names the path.
pub fn kit_text(name: &str) -> String {
    let path = kit_path(name);
    std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "cannot read the kit file {}: {error}; the kit is part of the SDK archive (docs/wrapper-guide/nettest)",
            path.display()
        )
    })
}

/// Reads a kit file as JSON; the message of a failure names the path.
pub fn kit_json(name: &str) -> Value {
    serde_json::from_str(&kit_text(name))
        .unwrap_or_else(|error| panic!("the kit file {} is not JSON: {error}", kit_path(name).display()))
}

/// A 64-bit value of the kit: a hexadecimal string with the prefix `0x`.
pub fn kit_hex(value: &Value) -> u64 {
    let text = value
        .as_str()
        .unwrap_or_else(|| panic!("a hexadecimal string was expected: {value}"));
    let digits = text
        .strip_prefix("0x")
        .unwrap_or_else(|| panic!("the prefix 0x was expected: {text}"));
    u64::from_str_radix(digits, 16).unwrap_or_else(|error| panic!("not a hexadecimal value: {text}: {error}"))
}
