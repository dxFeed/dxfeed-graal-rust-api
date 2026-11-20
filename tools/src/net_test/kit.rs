//! The kit of the NetTest specification: the vectors that the SDK archive ships next to the specification. The build
//! script of the tools hands over its directory.

use std::path::PathBuf;

use serde_json::Value;

/// The path of a kit file.
fn kit_path(name: &str) -> PathBuf {
    PathBuf::from(env!("NETTEST_KIT_DIR")).join(name)
}

/// Reads a kit file as JSON; the message of a failure names the path.
pub fn kit_json(name: &str) -> Value {
    let path = kit_path(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "cannot read the kit file {}: {error}; the kit is part of the SDK archive (docs/wrapper-guide/nettest)",
            path.display()
        )
    });
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("the kit file {} is not JSON: {error}", path.display()))
}
