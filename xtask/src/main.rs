//! Maintainer tasks for the dxFeed Graal Rust API workspace.
//!
//! `cargo run -p xtask -- gen-bindings <sdk dir>` regenerates `dxfeed-graal-sys/src/bindings.rs` from the headers of an
//! unpacked SDK archive. It needs libclang (set `LIBCLANG_PATH` if the right one is not found first).

use std::path::{Path, PathBuf};
use std::process::ExitCode;

const SDK_VERSION: &str = "3.12.0";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [task, sdk_dir] if task == "gen-bindings" => match gen_bindings(Path::new(sdk_dir)) {
            Ok(path) => {
                println!("wrote {}", path.display());
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("error: {error}");
                ExitCode::FAILURE
            }
        },
        _ => {
            eprintln!("usage: cargo run -p xtask -- gen-bindings <unpacked SDK {SDK_VERSION} dir>");
            ExitCode::from(2)
        }
    }
}

fn gen_bindings(sdk_dir: &Path) -> Result<PathBuf, String> {
    let header = sdk_dir.join("dxfg_v4_api.h");
    if !header.is_file() {
        return Err(format!("{} not found", header.display()));
    }
    // The event queue API is not part of SDK 3.12.0; a header set that has it is another version.
    if sdk_dir.join("dxfg_v4_event_queue.h").exists() {
        return Err(format!(
            "{} is not SDK {SDK_VERSION}: it has dxfg_v4_event_queue.h",
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
        .default_enum_style(bindgen::EnumVariation::Consts)
        .prepend_enum_name(false)
        .layout_tests(true)
        .derive_default(true)
        .rustfmt_configuration_file(Some(Path::new(env!("CARGO_MANIFEST_DIR")).join("../rustfmt.toml")))
        .generate()
        .map_err(|error| format!("bindgen failed: {error}"))?;

    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../dxfeed-graal-sys/src/bindings.rs");
    let text = format!(
        "// Generated from the headers of dxFeed Graal Native SDK {SDK_VERSION} by\n\
         // `cargo run -p xtask -- gen-bindings <sdk dir>`. Do not edit.\n\n{bindings}"
    );
    std::fs::write(&out, text).map_err(|error| format!("cannot write {}: {error}", out.display()))?;
    Ok(out.canonicalize().unwrap_or(out))
}
