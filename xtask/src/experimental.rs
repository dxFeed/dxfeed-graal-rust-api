//! `check-experimental`: the stable API uses no experimental SDK item outside the allowlist.
//!
//! The SDK marks its experimental functions, structures and constants with `@experimental` in their documentation,
//! which bindgen keeps in the bindings. The crate is expanded without the feature `experimental`, so the expansion
//! holds only the stable API and its private code; every experimental item that its code (not its documentation)
//! names must be in `experimental-allowlist.txt`, and every entry there must still be experimental and named.

use std::collections::BTreeSet;
use std::fs;
use std::process::Command;

/// The experimental SDK items that the stable API may use, one per line; `#` starts a comment.
const ALLOWLIST: &str = include_str!("../experimental-allowlist.txt");

pub fn check_experimental() -> Result<String, String> {
    let root = crate::workspace_root();
    let path = root.join("dxfeed-graal-sys/src/bindings.rs");
    let bindings = fs::read_to_string(&path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let experimental = experimental_items(&bindings);
    if experimental.is_empty() {
        return Err(format!(
            "no @experimental item in {}: has the format changed?",
            path.display()
        ));
    }
    let allowlist = allowlist(ALLOWLIST)?;
    let used = used(&code(&expand()?), &experimental);
    let errors = check(&experimental, &allowlist, &used);
    if errors.is_empty() {
        Ok(format!(
            "the stable API uses {} of {} experimental SDK items, all in the allowlist",
            used.len(),
            experimental.len()
        ))
    } else {
        Err(errors.join("\n"))
    }
}

/// The items whose documentation has `@experimental`. Bindgen writes one `#[doc]` line before the item, perhaps with
/// other attributes between them.
fn experimental_items(bindings: &str) -> BTreeSet<String> {
    const KINDS: [&str; 7] = ["fn ", "struct ", "union ", "enum ", "type ", "const ", "static "];
    let mut items = BTreeSet::new();
    let mut experimental = false;
    for line in bindings.lines().map(str::trim_start) {
        if line.starts_with("#[doc = ") {
            experimental |= line.contains("@experimental");
        } else if let Some(rest) = line.strip_prefix("pub ") {
            let name = KINDS
                .iter()
                .find_map(|kind| rest.strip_prefix(kind))
                .and_then(|rest| rest.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).next());
            if experimental && let Some(name) = name.filter(|name| !name.is_empty()) {
                items.insert(name.to_owned());
            }
            experimental = false;
        }
    }
    items
}

fn allowlist(text: &str) -> Result<BTreeSet<String>, String> {
    let mut names = BTreeSet::new();
    for line in text.lines() {
        let name = line.split_once('#').map_or(line, |(name, _)| name).trim();
        if !name.is_empty() && !names.insert(name.to_owned()) {
            return Err(format!("{name} is twice in the allowlist"));
        }
    }
    Ok(names)
}

/// The crate expanded without the feature `experimental` but with the other features. `-Zunpretty` needs
/// `RUSTC_BOOTSTRAP`; `DOCS_RS` spares the SDK, which the expansion does not link.
fn expand() -> Result<String, String> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let target = crate::workspace_root().join("target/xtask-expand");
    let output = Command::new(cargo)
        .current_dir(crate::workspace_root())
        .env("RUSTC_BOOTSTRAP", "1")
        .env("DOCS_RS", "1")
        .args([
            "rustc",
            "--quiet",
            "--locked",
            "-p",
            "dxfeed-graal",
            "--lib",
            "--features",
            "stream",
        ])
        .arg("--target-dir")
        .arg(target)
        .args(["--", "-Zunpretty=expanded"])
        .output()
        .map_err(|error| format!("cannot run cargo: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo rustc failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    String::from_utf8(output.stdout).map_err(|_| "the expansion is not valid UTF-8".to_owned())
}

/// The source without its documentation, which may name SDK items that the code does not use. The expansion writes
/// each doc comment line as a `#[doc = "..."]` line.
fn code(source: &str) -> String {
    source
        .lines()
        .filter(|line| {
            let line = line.trim_start();
            !["#[doc", "#![doc", "///", "//!"]
                .iter()
                .any(|doc| line.starts_with(doc))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The experimental items that the source names as whole identifiers.
fn used(source: &str, experimental: &BTreeSet<String>) -> BTreeSet<String> {
    experimental
        .iter()
        .filter(|name| names(source, name))
        .cloned()
        .collect()
}

fn names(source: &str, name: &str) -> bool {
    let ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
    source
        .match_indices(name)
        .any(|(at, _)| !source[..at].ends_with(ident) && !source[at + name.len()..].starts_with(ident))
}

fn check(experimental: &BTreeSet<String>, allowlist: &BTreeSet<String>, used: &BTreeSet<String>) -> Vec<String> {
    let mut errors = Vec::new();
    for name in used.difference(allowlist) {
        errors.push(format!(
            "the stable API uses the experimental SDK item {name}: put the code that uses it behind the feature \
             experimental, or add {name} to xtask/experimental-allowlist.txt with the reason"
        ));
    }
    for name in allowlist.difference(experimental) {
        errors.push(format!(
            "{name} is in the allowlist but not experimental in the bindings: remove it"
        ));
    }
    for name in allowlist
        .intersection(experimental)
        .filter(|name| !used.contains(*name))
    {
        errors.push(format!(
            "{name} is in the allowlist but the stable API does not use it: remove it"
        ));
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    const BINDINGS: &str = r#"
unsafe extern "C" {
    #[doc = " @brief Stable.\n @since 3.14.0"]
    pub fn dxfg_v4_A_stable(thread: *mut graal_isolatethread_t) -> i32;
}
unsafe extern "C" {
    #[doc = " @brief Experimental.\n @experimental\n @since 3.14.0"]
    pub fn dxfg_v4_A_experimental(thread: *mut graal_isolatethread_t) -> i32;
}
#[doc = " @brief An experimental event.\n @experimental"]
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct dxfg_event_t {
    pub x: i32,
}
#[doc = " @experimental"]
pub const DXFG_V4_FEATURE_X: &[u8; 2] = b"x\0";
#[doc = " @experimental"]
pub type dxfg_v4_func = ::std::option::Option<unsafe extern "C" fn()>;
#[doc = " A stable structure."]
pub struct dxfg_stable_t {
    pub x: i32,
}
unsafe extern "C" {
    pub fn dxfg_v4_A_experimentalMillis(thread: *mut graal_isolatethread_t) -> i32;
}
"#;

    fn set(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|&name| name.to_owned()).collect()
    }

    #[test]
    fn reads_the_experimental_items() {
        assert_eq!(
            experimental_items(BINDINGS),
            set(&[
                "DXFG_V4_FEATURE_X",
                "dxfg_event_t",
                "dxfg_v4_A_experimental",
                "dxfg_v4_func"
            ])
        );
    }

    #[test]
    fn the_allowlist_is_in_the_bindings() {
        let path = crate::workspace_root().join("dxfeed-graal-sys/src/bindings.rs");
        let experimental = experimental_items(&fs::read_to_string(path).unwrap());
        let allowlist = allowlist(ALLOWLIST).unwrap();
        assert!(!allowlist.is_empty());
        assert_eq!(
            allowlist.difference(&experimental).collect::<Vec<_>>(),
            Vec::<&String>::new()
        );
    }

    #[test]
    fn reads_the_allowlist() {
        assert_eq!(
            allowlist("# group\ndxfg_a # reason\n\n  dxfg_b\n").unwrap(),
            set(&["dxfg_a", "dxfg_b"])
        );
        assert!(allowlist("dxfg_a\ndxfg_a\n").is_err());
    }

    #[test]
    fn matches_whole_identifiers_in_the_code() {
        let experimental = set(&["dxfg_v4_A_experimental"]);
        assert_eq!(used("sys::dxfg_v4_A_experimental(t)", &experimental).len(), 1);
        assert!(used("sys::dxfg_v4_A_experimentalMillis(t)", &experimental).is_empty());
        assert!(used("sys::xdxfg_v4_A_experimental(t)", &experimental).is_empty());
        let documented = "    #[doc = \" Wraps `dxfg_v4_A_experimental`.\"]\n    fn f() {}\n";
        assert!(used(&code(documented), &experimental).is_empty());
    }

    #[test]
    fn fails_on_a_use_outside_the_allowlist() {
        let experimental = set(&["dxfg_a", "dxfg_b", "dxfg_c"]);
        let errors = check(&experimental, &set(&["dxfg_a"]), &set(&["dxfg_a", "dxfg_b"]));
        assert_eq!(errors.len(), 1);
        assert!(
            errors[0].contains("uses the experimental SDK item dxfg_b"),
            "{errors:?}"
        );
    }

    #[test]
    fn fails_on_stale_entries() {
        let experimental = set(&["dxfg_a", "dxfg_b"]);
        let errors = check(
            &experimental,
            &set(&["dxfg_a", "dxfg_b", "dxfg_gone"]),
            &set(&["dxfg_a"]),
        );
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(
            errors[0].contains("dxfg_gone is in the allowlist but not experimental"),
            "{errors:?}"
        );
        assert!(
            errors[1].contains("dxfg_b is in the allowlist but the stable API does not use it"),
            "{errors:?}"
        );
    }
}
