//! Maintainer tasks for the dxFeed Graal Rust API workspace.
//!
//! `cargo run -p xtask -- gen-bindings <sdk dir>` regenerates `dxfeed-graal-sys/src/bindings.rs` from the headers of an
//! unpacked SDK archive. It needs libclang (set `LIBCLANG_PATH` if the right one is not found first).
//!
//! `cargo run -p xtask -- check-deploy` builds `samples/deploy` and checks that its executable runs outside cargo from
//! a directory holding only the executable and the deployed SDK library, and fails without the library.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

const SDK_VERSION: &str = "3.13.0";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.as_slice() {
        [task, sdk_dir] if task == "gen-bindings" => {
            gen_bindings(Path::new(sdk_dir)).map(|path| format!("wrote {}", path.display()))
        }
        [task] if task == "check-deploy" => check_deploy(),
        _ => {
            eprintln!("usage: cargo run -p xtask -- gen-bindings <unpacked SDK {SDK_VERSION} dir>");
            eprintln!("       cargo run -p xtask -- check-deploy");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn gen_bindings(sdk_dir: &Path) -> Result<PathBuf, String> {
    let header = sdk_dir.join("dxfg_v4_api.h");
    if !header.is_file() {
        return Err(format!("{} not found", header.display()));
    }
    // The event queue API is new in SDK 3.13.0; a header set without it is an older version.
    if !sdk_dir.join("dxfg_v4_event_queue.h").is_file() {
        return Err(format!(
            "{} is not SDK {SDK_VERSION}: it has no dxfg_v4_event_queue.h",
            sdk_dir.display()
        ));
    }
    let header = header.to_str().ok_or("SDK path is not valid UTF-8")?;

    let bindings = bindgen::Builder::default()
        .header(header)
        .clang_arg(format!("-I{}", sdk_dir.display()))
        // The one width-dependent SDK type, `__graal_uword`, is `unsigned long long` on 64-bit Windows and maps to
        // `u64` everywhere; on other targets it is `unsigned long`, which bindgen maps to `c_ulong` (32-bit on Windows).
        // Generating for 64-bit Windows makes one file valid for every 64-bit target.
        .clang_arg("--target=x86_64-pc-windows-msvc")
        .allowlist_function("dxfg_v4_.*|graal_.*")
        .allowlist_type("dxfg_v4_.*|graal_.*")
        .allowlist_var("DXFG_.*")
        // Macros with a cast, such as `((int32_t)1)`, are evaluated by clang; bindgen alone skips them.
        .clang_macro_fallback()
        .default_enum_style(bindgen::EnumVariation::Consts)
        .prepend_enum_name(false)
        .layout_tests(true)
        .derive_default(true)
        .rustfmt_configuration_file(Some(workspace_root().join("rustfmt.toml")))
        .generate()
        .map_err(|error| format!("bindgen failed: {error}"))?;

    let out = workspace_root().join("dxfeed-graal-sys/src/bindings.rs");
    let text = format!(
        "// Generated from the headers of dxFeed Graal Native SDK {SDK_VERSION} by\n\
         // `cargo run -p xtask -- gen-bindings <sdk dir>`. Do not edit.\n\n{bindings}"
    );
    fs::write(&out, text).map_err(|error| format!("cannot write {}: {error}", out.display()))?;
    Ok(out.canonicalize().unwrap_or(out))
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

const SAMPLE: &str = "deploy-sample";
const SAMPLE_OUTPUT: &str = "Created a Feed endpoint with a quote subscription";
const LIBRARY: &str = if cfg!(windows) {
    "DxFeedGraalNativeSdk.dll"
} else if cfg!(target_os = "macos") {
    "libDxFeedGraalNativeSdk.dylib"
} else {
    "libDxFeedGraalNativeSdk.so"
};

fn check_deploy() -> Result<String, String> {
    let workspace = workspace_root();
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let build = || {
        let output = Command::new(&cargo)
            // The rebuild check looks for the plain `Compiling` line, which colors and quiet mode would hide.
            .args(["build", "-p", SAMPLE, "--color", "never"])
            .env_remove("CARGO_TERM_QUIET")
            .current_dir(&workspace)
            .output()
            .map_err(|error| format!("cannot run cargo: {error}"))?;
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        if output.status.success() {
            Ok(stderr)
        } else {
            Err(format!("cargo build -p {SAMPLE} failed:\n{stderr}"))
        }
    };
    build()?;

    let profile = target_dir(&cargo, &workspace)?.join("debug");
    let exe = profile.join(format!("{SAMPLE}{}", std::env::consts::EXE_SUFFIX));
    let library = profile.join(LIBRARY);
    for file in [&exe, &library] {
        if !file.is_file() {
            return Err(format!("{} is missing after the build", file.display()));
        }
    }

    suppress_error_dialogs();
    let temp = std::env::temp_dir().join(format!("dxfeed-graal-check-deploy-{}", std::process::id()));
    let result = run_outside_cargo(&exe, &library, &temp);
    let _ = fs::remove_dir_all(&temp);
    result?;
    rebuild_keeps_library(&library, &workspace.join("samples/deploy/build.rs"), build)?;

    Ok(format!(
        "check-deploy passed: {} runs with {LIBRARY} next to it, fails without it, and a rebuild keeps the library",
        exe.display()
    ))
}

/// Runs the executable from a directory with the library, where it must succeed, and from one without, where it
/// must fail. Loader variables that could find the library elsewhere are removed.
fn run_outside_cargo(exe: &Path, library: &Path, temp: &Path) -> Result<(), String> {
    let copy = |from: &Path, dir: &Path| {
        fs::create_dir_all(dir)
            .and_then(|()| fs::copy(from, dir.join(from.file_name().unwrap_or_default())))
            .map_err(|error| format!("cannot copy {} to {}: {error}", from.display(), dir.display()))
    };
    let (with_library, alone) = (temp.join("with-library"), temp.join("alone"));
    copy(exe, &with_library)?;
    copy(library, &with_library)?;
    copy(exe, &alone)?;
    let exe_name = exe.file_name().unwrap_or_default();

    let (succeeded, output) = run_sanitized(&with_library.join(exe_name))?;
    if !succeeded || !output.contains(SAMPLE_OUTPUT) {
        return Err(format!(
            "the deployed executable failed in {}:\n{output}",
            with_library.display()
        ));
    }
    let (succeeded, output) = run_sanitized(&alone.join(exe_name))?;
    if succeeded {
        return Err(format!(
            "the executable ran without the library in {}:\n{output}",
            alone.display()
        ));
    }
    Ok(())
}

/// Touches the sample's build script, which makes cargo rerun it, and checks that the rebuild leaves the deployed
/// library untouched. A copy keeps the size and modification time, so the deployed library gets a changed last
/// byte, which only a repeated copy would undo, and gets its original bytes back afterwards.
fn rebuild_keeps_library(
    library: &Path,
    build_rs: &Path,
    build: impl Fn() -> Result<String, String>,
) -> Result<(), String> {
    let error = |error: std::io::Error| format!("cannot rewrite {}: {error}", library.display());
    let original = fs::read(library).map_err(error)?;
    let modified = fs::metadata(library).and_then(|m| m.modified()).map_err(error)?;
    let rewrite = |bytes: &[u8]| {
        fs::write(library, bytes)
            .and_then(|()| fs::File::options().write(true).open(library)?.set_modified(modified))
            .map_err(error)
    };
    let mut marked = original.clone();
    if let Some(last) = marked.last_mut() {
        *last ^= 0xFF;
    }
    rewrite(&marked)?;

    let rebuild = || {
        fs::File::options()
            .write(true)
            .open(build_rs)
            .and_then(|file| file.set_modified(std::time::SystemTime::now()))
            .map_err(|error| format!("cannot touch {}: {error}", build_rs.display()))?;
        let stderr = build()?;
        if !stderr.contains(&format!("Compiling {SAMPLE}")) {
            return Err(format!(
                "the rebuild after touching build.rs did not compile {SAMPLE}:\n{stderr}"
            ));
        }
        if fs::read(library).map_err(error)? != marked {
            return Err(format!(
                "the rebuild copied {} again instead of skipping it",
                library.display()
            ));
        }
        Ok(())
    };
    let result = rebuild();
    rewrite(&original)?;
    result
}

/// Runs `exe` in its own directory without loader paths that hold the library, and returns whether it succeeded
/// and its combined output. Kills it after a minute.
fn run_sanitized(exe: &Path) -> Result<(bool, String), String> {
    let dir = exe.parent().unwrap_or(Path::new("."));
    let mut command = Command::new(exe);
    command
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for var in ["LD_LIBRARY_PATH", "DYLD_LIBRARY_PATH", "DYLD_FALLBACK_LIBRARY_PATH"] {
        command.env_remove(var);
    }
    if let Some(path) = std::env::var_os("PATH") {
        let kept = std::env::split_paths(&path).filter(|entry| !entry.join(LIBRARY).exists());
        command.env("PATH", std::env::join_paths(kept).map_err(|error| error.to_string())?);
    }

    let mut child = command
        .spawn()
        .map_err(|error| format!("cannot start {}: {error}", exe.display()))?;
    let deadline = Instant::now() + Duration::from_secs(60);
    while child.try_wait().map_err(|error| error.to_string())?.is_none() {
        if Instant::now() > deadline {
            let _ = child.kill();
            return Err(format!("{} did not exit within a minute", exe.display()));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let output = child.wait_with_output().map_err(|error| error.to_string())?;
    let text = String::from_utf8_lossy(&output.stdout).into_owned() + &String::from_utf8_lossy(&output.stderr);
    Ok((output.status.success(), text))
}

/// Keeps Windows from showing a "DLL not found" dialog for the executable started without its library: sets the error
/// mode of this process, which its children inherit.
fn suppress_error_dialogs() {
    #[cfg(windows)]
    {
        const SEM_FAILCRITICALERRORS: u32 = 0x0001;
        unsafe extern "system" {
            fn SetErrorMode(mode: u32) -> u32;
        }
        // SAFETY: SetErrorMode only changes a flag of this process and has no memory preconditions.
        unsafe { SetErrorMode(SEM_FAILCRITICALERRORS) };
    }
}

/// The target directory from `cargo metadata`, which honours `CARGO_TARGET_DIR` and the cargo configuration.
fn target_dir(cargo: &std::ffi::OsStr, workspace: &Path) -> Result<PathBuf, String> {
    let output = Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(workspace)
        .output()
        .map_err(|error| format!("cannot run cargo metadata: {error}"))?;
    let json = String::from_utf8_lossy(&output.stdout);
    // A JSON string without escapes other than doubled backslashes, which is what a path gives.
    let key = "\"target_directory\":\"";
    let start = json.find(key).ok_or("cargo metadata has no target_directory")? + key.len();
    let end = json[start..].find('"').ok_or("cargo metadata is truncated")? + start;
    Ok(PathBuf::from(json[start..end].replace("\\\\", "\\")))
}
