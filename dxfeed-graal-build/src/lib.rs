//! Deploys the dxFeed Graal Native SDK library next to an application, so that its executables run outside cargo.
//!
//! The SDK is a dynamic library. `cargo run` and `cargo test` find it on their own, but an executable started outside
//! cargo needs the library next to it. Add this crate to the `[build-dependencies]` of the application that depends
//! on `dxfeed-graal`, and call [`deploy`] from its build script:
//!
//! ```no_run
//! // build.rs
//! fn main() {
//!     dxfeed_graal_build::deploy().unwrap();
//! }
//! ```
//!
//! [`deploy`] copies the library into the cargo profile directory (`target/debug`, `target/release`, or
//! `target/<triple>/<profile>`), where the application's executables are written, and makes Linux and macOS
//! executables look for it in their own directory (an rpath of `$ORIGIN` or `@loader_path`). Windows looks there
//! first anyway. Copy the executable together with the library to run it elsewhere.
//!
//! Examples and test executables are not covered: they live in other directories and run through cargo. The profile
//! directory is derived from the build script's `OUT_DIR`, which cargo does not document as stable. Projects that set
//! `build.build-dir`, or that place executables elsewhere, copy the file from [`library_path`] themselves. macOS has
//! not been verified yet.

#![allow(clippy::needless_doctest_main, reason = "the example is a whole build script")]

use std::env;
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Set by the build script of `dxfeed-graal` for the build scripts of its direct dependents.
const LIB_DIR_VAR: &str = "DEP_DXFEED_GRAAL_LIB_DIR";

/// An error that stops the deployment, with a message for the build output.
#[derive(Debug)]
pub struct Error(String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

/// The result of this crate's functions.
pub type Result<T> = std::result::Result<T, Error>;

/// Returns the path of the SDK runtime library (`DxFeedGraalNativeSdk.dll`, `libDxFeedGraalNativeSdk.so` or
/// `libDxFeedGraalNativeSdk.dylib`) for the target being built.
///
/// Call it from the build script of a package that depends on `dxfeed-graal`.
pub fn library_path() -> Result<PathBuf> {
    let dir = lib_dir(env::var_os(LIB_DIR_VAR))?;
    Ok(dir.join(library_file_name(&target_os()?)?))
}

/// Copies the SDK runtime library next to the executables of the package whose build script calls it, and gives
/// those executables an rpath to their own directory on Linux and macOS. Does nothing on docs.rs.
///
/// The copy is skipped when the library there is already the same, so a rebuild does not fail on Windows while the
/// application runs and holds the library open.
pub fn deploy() -> Result<()> {
    if env::var_os("DOCS_RS").is_some() {
        return Ok(());
    }
    println!("cargo::rerun-if-env-changed={LIB_DIR_VAR}");
    let source = library_path()?;
    let out_dir =
        env::var_os("OUT_DIR").ok_or_else(|| Error("OUT_DIR is not set; call deploy() from build.rs".into()))?;
    let dest = profile_dir(Path::new(&out_dir))?.join(source.file_name().unwrap_or_default());

    copy_if_changed(&source, &dest)
        .map_err(|e| Error(format!("cannot copy {} to {}: {e}", source.display(), dest.display())))?;
    println!("cargo::rerun-if-changed={}", source.display());
    println!("cargo::rerun-if-changed={}", dest.display());
    if let Some(arg) = rpath_arg(&target_os()?) {
        println!("cargo::rustc-link-arg-bins={arg}");
    }
    Ok(())
}

fn target_os() -> Result<String> {
    env::var("CARGO_CFG_TARGET_OS").map_err(|_| Error("CARGO_CFG_TARGET_OS is not set; call this from build.rs".into()))
}

fn lib_dir(value: Option<OsString>) -> Result<PathBuf> {
    value.map(PathBuf::from).ok_or_else(|| {
        Error(format!(
            "{LIB_DIR_VAR} is not set: the package whose build script calls dxfeed-graal-build must depend on \
             dxfeed-graal"
        ))
    })
}

/// File name of the runtime library on `target_os` (a `CARGO_CFG_TARGET_OS` value).
fn library_file_name(target_os: &str) -> Result<&'static str> {
    match target_os {
        "windows" => Ok("DxFeedGraalNativeSdk.dll"),
        "linux" => Ok("libDxFeedGraalNativeSdk.so"),
        "macos" => Ok("libDxFeedGraalNativeSdk.dylib"),
        other => Err(Error(format!(
            "the dxFeed Graal Native SDK does not support target OS {other}"
        ))),
    }
}

/// The linker argument that makes executables look for libraries in their own directory, if `target_os` needs one.
fn rpath_arg(target_os: &str) -> Option<&'static str> {
    match target_os {
        "linux" => Some("-Wl,-rpath,$ORIGIN"),
        "macos" => Some("-Wl,-rpath,@loader_path"),
        _ => None,
    }
}

/// The profile directory for a build script's `OUT_DIR`, which cargo lays out as
/// `<target>/[<triple>/]<profile>/build/<package>-<hash>/out`. Any other shape is an error rather than a guess.
fn profile_dir(out_dir: &Path) -> Result<PathBuf> {
    out_dir
        .ancestors()
        .nth(2)
        .filter(|build| build.file_name().is_some_and(|name| name == "build"))
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| {
            Error(format!(
                "cannot find the profile directory from OUT_DIR {}: expected <profile>/build/<package>/out; copy \
                 the file from dxfeed_graal_build::library_path() yourself",
                out_dir.display()
            ))
        })
}

/// Copies `from` to `to` unless `to` already has the same size and modification time. Gives the copy the source's
/// modification time, which `fs::copy` keeps only on Windows, so the next call can skip it. Returns whether it copied.
///
/// The build scripts of several packages of a workspace may deploy into the same directory at once, so the library is
/// copied into a file of this call and renamed into place: no one sees a partial copy, and two copies do not write one
/// file. When the rename fails but the destination is the same by then, another build has put it there, and that counts.
fn copy_if_changed(from: &Path, to: &Path) -> io::Result<bool> {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let source = fs::metadata(from)?;
    let modified = source.modified()?;
    let same = || {
        fs::metadata(to).is_ok_and(|dest| dest.len() == source.len() && dest.modified().is_ok_and(|m| m == modified))
    };
    if same() {
        return Ok(false);
    }
    let temp = to.with_file_name(format!(
        ".{}.{}-{}.tmp",
        to.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let copied = (|| {
        fs::copy(from, &temp)?;
        fs::File::options().write(true).open(&temp)?.set_modified(modified)?;
        fs::rename(&temp, to)
    })();
    match copied {
        Ok(()) => Ok(true),
        Err(error) => {
            let _ = fs::remove_file(&temp);
            if same() { Ok(false) } else { Err(error) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_names_follow_the_platform_conventions() {
        assert_eq!(library_file_name("windows").unwrap(), "DxFeedGraalNativeSdk.dll");
        assert_eq!(library_file_name("linux").unwrap(), "libDxFeedGraalNativeSdk.so");
        assert_eq!(library_file_name("macos").unwrap(), "libDxFeedGraalNativeSdk.dylib");
        assert!(
            library_file_name("freebsd")
                .unwrap_err()
                .to_string()
                .contains("freebsd")
        );
    }

    #[test]
    fn rpath_points_at_the_executable_directory() {
        assert_eq!(rpath_arg("windows"), None);
        assert_eq!(rpath_arg("linux"), Some("-Wl,-rpath,$ORIGIN"));
        assert_eq!(rpath_arg("macos"), Some("-Wl,-rpath,@loader_path"));
    }

    #[test]
    fn profile_dir_is_found_with_and_without_a_target_triple() {
        let host = Path::new("target").join("debug");
        assert_eq!(
            profile_dir(&host.join("build").join("app-1a2b").join("out")).unwrap(),
            host
        );
        let cross = Path::new("target").join("x86_64-unknown-linux-gnu").join("release");
        assert_eq!(
            profile_dir(&cross.join("build").join("app-1a2b").join("out")).unwrap(),
            cross
        );
    }

    #[test]
    fn unexpected_out_dir_shapes_are_errors() {
        assert!(profile_dir(Path::new("out")).is_err());
        assert!(profile_dir(&Path::new("build").join("out")).is_err());
        let error = profile_dir(
            &Path::new("target")
                .join("debug")
                .join("other")
                .join("app-1a2b")
                .join("out"),
        );
        assert!(error.unwrap_err().to_string().contains("library_path()"));
    }

    #[test]
    fn missing_lib_dir_names_the_dependency() {
        assert!(
            lib_dir(None)
                .unwrap_err()
                .to_string()
                .contains("depend on dxfeed-graal")
        );
        assert_eq!(lib_dir(Some("sdk".into())).unwrap(), PathBuf::from("sdk"));
    }

    #[test]
    fn copy_is_skipped_when_the_destination_is_the_same() {
        let dir = env::temp_dir().join(format!("dxfeed-graal-build-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let (from, to) = (dir.join("from"), dir.join("to"));
        fs::write(&from, b"library").unwrap();
        let _ = fs::remove_file(&to);

        assert!(copy_if_changed(&from, &to).unwrap(), "a missing destination is copied");
        assert!(
            !copy_if_changed(&from, &to).unwrap(),
            "an identical destination is skipped"
        );
        fs::write(&from, b"a new library").unwrap();
        assert!(copy_if_changed(&from, &to).unwrap(), "a changed source is copied");
        assert_eq!(fs::read(&to).unwrap(), b"a new library");
        fs::write(&from, b"A NEW LIBRARY").unwrap();
        fs::File::options()
            .write(true)
            .open(&from)
            .unwrap()
            .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000))
            .unwrap();
        assert!(
            copy_if_changed(&from, &to).unwrap(),
            "a source of the same size with another modification time is copied"
        );
        assert_eq!(fs::read(&to).unwrap(), b"A NEW LIBRARY");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn concurrent_copies_leave_one_whole_library() {
        let dir = env::temp_dir().join(format!("dxfeed-graal-build-concurrent-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let (from, to) = (dir.join("from"), dir.join("to"));
        // As big as the SDK library, so the copies overlap.
        let library = vec![7u8; 32 << 20];
        fs::write(&from, &library).unwrap();
        let _ = fs::remove_file(&to);

        let start = std::sync::Barrier::new(8);
        std::thread::scope(|scope| {
            let copies: Vec<_> = (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        start.wait();
                        copy_if_changed(&from, &to)
                    })
                })
                .collect();
            for copy in copies {
                copy.join().unwrap().unwrap();
            }
        });
        assert!(fs::read(&to).unwrap() == library);
        let names: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 2, "temporary files are left: {names:?}");

        fs::remove_dir_all(&dir).unwrap();
    }
}
