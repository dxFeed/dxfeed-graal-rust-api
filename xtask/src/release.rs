//! Release tasks: the tag and notes of a release, and the archives of the Tools and the samples.
//!
//! The archives follow the C++ API: `dxFeedGraalRustApi-<tag>-<arch>-<os>-<Tools|Samples>`, a zip on Windows and a
//! gzipped tar elsewhere, holding one folder of that name with `bin/` (the executables and the SDK library) and the
//! license.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{LIBRARY, run_sanitized, suppress_error_dialogs, target_dir, workspace_root};

const PRODUCT: &str = "dxFeedGraalRustApi";
const NOTES_FILE: &str = "ReleaseNotes.md";
/// The one `dxfeed.system.properties` of the samples archive, the demo feed, as the C++ API installs it.
const SAMPLE_PROPERTIES: &str = "samples/API/PrintQuoteEventsSample/dxfeed.system.properties";
/// The sample that checks the samples archive: it runs without a network and writes a tape file.
const CHECK_SAMPLE: &str = "WriteTapeFileSample";
const CHECK_SAMPLE_OUTPUT_FILE: &str = "WriteTapeFile.out.txt";
const CHECK_SAMPLE_OUTPUT: &str = "TEST1";

/// A release tag: `v<major>.<minor>.<patch>` with an optional `-<suffix>`.
#[derive(Debug, PartialEq)]
pub struct Tag<'a> {
    pub name: &'a str,
    /// The tag without its `v`: `<major>.<minor>.<patch>[-<suffix>]`, the workspace version of the release and the
    /// heading of its release notes.
    pub version: &'a str,
}

pub fn parse_tag(name: &str) -> Result<Tag<'_>, String> {
    let error = || format!("{name} is not a release tag v<major>.<minor>.<patch>[-<suffix>]");
    let rest = name.strip_prefix('v').ok_or_else(error)?;
    let (numbers, suffix) = match rest.split_once('-') {
        Some((version, suffix)) => (version, Some(suffix)),
        None => (rest, None),
    };
    let numbers: Vec<&str> = numbers.split('.').collect();
    let number = |part: &&str| {
        !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()) && (*part == "0" || !part.starts_with('0'))
    };
    if numbers.len() != 3 || !numbers.iter().all(number) {
        return Err(error());
    }
    let suffix_char = |c: char| c.is_ascii_alphanumeric() || c == '.' || c == '-';
    if suffix.is_some_and(|suffix| suffix.is_empty() || !suffix.chars().all(suffix_char)) {
        return Err(error());
    }
    Ok(Tag { name, version: rest })
}

/// The `version` of `[workspace.package]` in the root manifest.
pub fn workspace_version(manifest: &str) -> Result<String, String> {
    let mut in_package = false;
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            in_package = line == "[workspace.package]";
        } else if in_package && let Some(version) = string_value(line, "version") {
            return Ok(version.to_owned());
        }
    }
    Err("the root Cargo.toml has no [workspace.package] version".into())
}

/// Checks that the tag's version is the workspace version.
pub fn check_version(tag: &Tag, workspace_version: &str) -> Result<(), String> {
    if tag.version == workspace_version {
        Ok(())
    } else {
        Err(format!(
            "the tag {} does not match the workspace version {workspace_version} in Cargo.toml",
            tag.name
        ))
    }
}

/// The text of the `## v<version>` section of the release notes, without its heading.
pub fn notes_section(notes: &str, version: &str) -> Result<String, String> {
    let heading = format!("## v{version}");
    let mut lines = notes.lines().skip_while(|line| line.trim_end() != heading);
    if lines.next().is_none() {
        return Err(format!("{NOTES_FILE} has no section {heading}"));
    }
    let section: Vec<&str> = lines.take_while(|line| !line.starts_with("## ")).collect();
    let text = section.join("\n").trim().to_owned();
    if text.is_empty() {
        return Err(format!("the section {heading} of {NOTES_FILE} is empty"));
    }
    Ok(text + "\n")
}

/// The platform part of an archive name for a Rust `ARCH` and `OS`, as the C++ API names it.
pub fn platform(arch: &str, os: &str) -> Result<String, String> {
    let os = match os {
        "linux" | "windows" => os,
        "macos" => "osx",
        other => return Err(format!("no release archives for {other}")),
    };
    match arch {
        "x86_64" | "aarch64" => Ok(format!("{arch}-{os}")),
        other => Err(format!("no release archives for {other}")),
    }
}

/// The name of an archive without its extension.
pub fn archive_base(tag: &str, platform: &str, kind: &str) -> String {
    format!("{PRODUCT}-{tag}-{platform}-{kind}")
}

/// The names of the `[[bin]]` targets of a manifest.
pub fn bin_names(manifest: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut in_bin = false;
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            in_bin = line == "[[bin]]";
        } else if in_bin && let Some(name) = string_value(line, "name") {
            names.push(name.to_owned());
        }
    }
    names
}

/// The value of a manifest line `key = "value"`, for the simple strings of this workspace's manifests.
fn string_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let value = line.strip_prefix(key)?.trim_start().strip_prefix('=')?.trim();
    value.strip_prefix('"')?.strip_suffix('"')
}

fn read(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|error| format!("cannot read {}: {error}", path.display()))
}

/// `release-tag`: the tag of the workspace version.
pub fn release_tag() -> Result<String, String> {
    let version = workspace_version(&read(&workspace_root().join("Cargo.toml"))?)?;
    Ok(format!("v{version}"))
}

/// `release-notes <tag>`: checks the tag against the workspace version and returns its notes.
pub fn release_notes(tag: &str) -> Result<String, String> {
    let root = workspace_root();
    let tag = parse_tag(tag)?;
    check_version(&tag, &workspace_version(&read(&root.join("Cargo.toml"))?)?)?;
    notes_section(&read(&root.join(NOTES_FILE))?, tag.version).map(|notes| notes.trim_end().to_owned())
}

/// `package <tag> <out dir>`: writes the Tools and Samples archives of this platform from the release build into
/// `out`, and checks that their unpacked executables run.
pub fn package(tag: &str, out: &Path) -> Result<String, String> {
    let root = workspace_root();
    let parsed = parse_tag(tag)?;
    check_version(&parsed, &workspace_version(&read(&root.join("Cargo.toml"))?)?)?;
    let platform = platform(std::env::consts::ARCH, std::env::consts::OS)?;
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let release = target_dir(&cargo, &root)?.join("release");
    let exe = |name: &str| release.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    let library = release.join(LIBRARY);

    let samples = bin_names(&read(&root.join("samples/Cargo.toml"))?);
    if !samples.iter().any(|name| name == CHECK_SAMPLE) {
        return Err(format!("samples/Cargo.toml has no {CHECK_SAMPLE}"));
    }
    let tools_files = vec![exe("Tools"), library.clone()];
    let mut samples_files: Vec<PathBuf> = samples.iter().map(|name| exe(name)).collect();
    samples_files.extend([library.clone(), root.join(SAMPLE_PROPERTIES)]);
    for file in tools_files.iter().chain(&samples_files) {
        if !file.is_file() {
            return Err(format!(
                "{} is missing: build first with cargo build --release --locked -p dxfeed-graal-tools -p \
                 dxfeed-graal-samples",
                file.display()
            ));
        }
    }

    fs::create_dir_all(out).map_err(|error| format!("cannot create {}: {error}", out.display()))?;
    // Not canonicalize: on Windows it gives a verbatim `\\?\` path, which reads badly in the messages.
    let out = std::path::absolute(out).map_err(|error| format!("cannot resolve {}: {error}", out.display()))?;
    suppress_error_dialogs();
    let mut written = Vec::new();
    for (kind, files, check) in [
        ("Tools", &tools_files, Check::Tools),
        ("Samples", &samples_files, Check::Sample),
    ] {
        let base = archive_base(tag, &platform, kind);
        let archive = write_archive(&root, &out, &base, files)?;
        let temp = std::env::temp_dir().join(format!("dxfeed-graal-package-{}-{kind}", std::process::id()));
        let checked = check_archive(&archive, &base, &temp, check);
        let _ = fs::remove_dir_all(&temp);
        checked?;
        written.push(archive.display().to_string());
    }
    Ok(format!("wrote and checked {}", written.join(", ")))
}

/// Stages `files` in `out/base/bin` with the license next to `bin`, archives the folder and removes it.
fn write_archive(root: &Path, out: &Path, base: &str, files: &[PathBuf]) -> Result<PathBuf, String> {
    let stage = out.join(base);
    let bin = stage.join("bin");
    let _ = fs::remove_dir_all(&stage);
    let copy = |from: &Path, dir: &Path| {
        fs::create_dir_all(dir)
            .and_then(|()| fs::copy(from, dir.join(from.file_name().unwrap_or_default())))
            .map_err(|error| format!("cannot copy {} to {}: {error}", from.display(), dir.display()))
    };
    for file in files {
        copy(file, &bin)?;
    }
    copy(&root.join("LICENSE"), &stage)?;

    let archive = out.join(if cfg!(windows) {
        format!("{base}.zip")
    } else {
        format!("{base}.tar.gz")
    });
    let _ = fs::remove_file(&archive);
    let mut command = Command::new(tar());
    // No AppleDouble `._*` entries for the extended attributes of the files on macOS.
    command.env("COPYFILE_DISABLE", "1");
    if cfg!(windows) {
        command.args(["-a", "-c", "-f"]);
    } else {
        command.args(["-c", "-z", "-f"]);
    }
    command.arg(&archive).arg("-C").arg(out).arg(base);
    let result = run(&mut command);
    let _ = fs::remove_dir_all(&stage);
    result?;
    Ok(archive)
}

#[derive(Clone, Copy)]
enum Check {
    /// `bin/Tools` without arguments prints its usage and succeeds.
    Tools,
    /// `bin/WriteTapeFileSample` writes its tape file, as in check-deploy.
    Sample,
}

/// Unpacks `archive` into `temp` and runs its check from there, in a working directory outside the archive folder.
fn check_archive(archive: &Path, base: &str, temp: &Path, check: Check) -> Result<(), String> {
    let _ = fs::remove_dir_all(temp);
    let work = temp.join("work");
    fs::create_dir_all(&work).map_err(|error| format!("cannot create {}: {error}", work.display()))?;
    run(Command::new(tar()).arg("-x").arg("-f").arg(archive).arg("-C").arg(temp))?;
    let bin = temp.join(base).join("bin");
    if !bin.join(LIBRARY).is_file() || !temp.join(base).join("LICENSE").is_file() {
        return Err(format!("{} has no bin/{LIBRARY} or LICENSE", archive.display()));
    }
    let name = match check {
        Check::Tools => "Tools",
        Check::Sample => CHECK_SAMPLE,
    };
    let exe = bin.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    let (succeeded, output) = run_sanitized(&exe, &work)?;
    let ok = succeeded
        && match check {
            Check::Tools => output.contains("Usage:"),
            Check::Sample => fs::read_to_string(work.join(CHECK_SAMPLE_OUTPUT_FILE))
                .is_ok_and(|text| text.contains(CHECK_SAMPLE_OUTPUT)),
        };
    if ok {
        Ok(())
    } else {
        Err(format!(
            "{} from {} failed:\n{output}",
            exe.display(),
            archive.display()
        ))
    }
}

/// The tar of the system: bsdtar on Windows, which also writes zip archives, rather than a GNU tar of Git on `PATH`.
fn tar() -> PathBuf {
    match std::env::var_os("SystemRoot") {
        Some(root) if cfg!(windows) => Path::new(&root).join("System32").join("tar.exe"),
        _ => PathBuf::from("tar"),
    }
}

fn run(command: &mut Command) -> Result<(), String> {
    let output = command
        .output()
        .map_err(|error| format!("cannot run {command:?}: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{command:?} failed:\n{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_tags_are_parsed() {
        assert_eq!(
            parse_tag("v0.1.0").unwrap(),
            Tag {
                name: "v0.1.0",
                version: "0.1.0"
            }
        );
        assert_eq!(parse_tag("v10.2.30-rc.1").unwrap().version, "10.2.30-rc.1");
        assert_eq!(parse_tag("v0.1.0-draft1").unwrap().version, "0.1.0-draft1");
        for wrong in [
            "0.1.0",
            "v0.1",
            "v0.1.0.1",
            "v01.1.0",
            "v0.1.x",
            "v0.1.0-",
            "v0.1.0-a b",
            "v",
        ] {
            assert!(parse_tag(wrong).is_err(), "{wrong}");
        }
    }

    #[test]
    fn the_workspace_version_comes_from_the_workspace_package() {
        let manifest = "[workspace]\nmembers = []\n\n[workspace.package]\nedition = \"2024\"\nversion = \"1.2.3\"\n";
        assert_eq!(workspace_version(manifest).unwrap(), "1.2.3");
        assert!(workspace_version("[package]\nversion = \"1.2.3\"\n").is_err());
        let tag = parse_tag("v1.2.3-rc.1").unwrap();
        assert!(check_version(&tag, "1.2.3-rc.1").is_ok());
        assert!(check_version(&tag, "1.2.3").unwrap_err().contains("1.2.3"));
        assert!(
            check_version(&parse_tag("v1.2.3").unwrap(), "1.2.4")
                .unwrap_err()
                .contains("1.2.4")
        );
    }

    #[test]
    fn the_real_manifests_parse() {
        let root = workspace_root();
        let version = workspace_version(&read(&root.join("Cargo.toml")).unwrap()).unwrap();
        assert!(parse_tag(&format!("v{version}")).is_ok());
        let samples = bin_names(&read(&root.join("samples/Cargo.toml")).unwrap());
        assert!(
            samples.len() > 20 && samples.iter().any(|name| name == CHECK_SAMPLE),
            "{samples:?}"
        );
        assert!(notes_section(&read(&root.join(NOTES_FILE)).unwrap(), &version).is_ok());
    }

    #[test]
    fn notes_sections_end_at_the_next_section() {
        let notes = "# Notes\n\n## v0.2.0\n\n* Two\n\n## v0.1.0\n\n* One\n* More\n\n## v0.0.1\n\n* Zero\n";
        assert_eq!(notes_section(notes, "0.1.0").unwrap(), "* One\n* More\n");
        assert_eq!(notes_section(notes, "0.0.1").unwrap(), "* Zero\n");
        assert!(notes_section(notes, "0.3.0").unwrap_err().contains("## v0.3.0"));
        assert!(
            notes_section("## v0.1.0\n\n## v0.0.1\n* Zero\n", "0.1.0")
                .unwrap_err()
                .contains("empty")
        );
        assert!(notes_section("## v0.1.00\n* x\n", "0.1.0").is_err());
    }

    #[test]
    fn archives_are_named_like_the_cxx_ones() {
        let names: Vec<String> = [
            ("x86_64", "linux"),
            ("aarch64", "linux"),
            ("x86_64", "windows"),
            ("x86_64", "macos"),
            ("aarch64", "macos"),
        ]
        .iter()
        .map(|(arch, os)| archive_base("v0.1.0", &platform(arch, os).unwrap(), "Tools"))
        .collect();
        assert_eq!(
            names,
            [
                "dxFeedGraalRustApi-v0.1.0-x86_64-linux-Tools",
                "dxFeedGraalRustApi-v0.1.0-aarch64-linux-Tools",
                "dxFeedGraalRustApi-v0.1.0-x86_64-windows-Tools",
                "dxFeedGraalRustApi-v0.1.0-x86_64-osx-Tools",
                "dxFeedGraalRustApi-v0.1.0-aarch64-osx-Tools",
            ]
        );
        assert!(platform("x86", "windows").is_err());
        assert!(platform("x86_64", "freebsd").is_err());
    }

    #[test]
    fn bin_names_come_from_bin_sections_only() {
        let manifest = "[package]\nname = \"pkg\"\n\n[[bin]]\nname = \"One\"\npath = \"a.rs\"\n\n[dependencies]\n\
                        name = \"x\"\n\n[[bin]]\npath = \"b.rs\"\nname = \"Two\"\n";
        assert_eq!(bin_names(manifest), ["One", "Two"]);
    }
}
