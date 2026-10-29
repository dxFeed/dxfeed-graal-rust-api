//! Gets dxFeed Graal Native SDK 3.12.0 for the target and links it.
//!
//! The SDK comes from, in order: nowhere when `DOCS_RS` is set (no link directives); the unpacked archive in
//! `DXFEED_GRAAL_SDK_DIR`, whose library is copied into `OUT_DIR`; the GitHub release archive, verified against a
//! pinned SHA-256 and unpacked into `OUT_DIR` (feature `download`). Linking always points at `OUT_DIR`, so cargo puts
//! the library on the loader path for `cargo run` and `cargo test`.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

const SDK_VERSION: &str = "3.12.0";
const LIB_NAME: &str = "DxFeedGraalNativeSdk";
const SDK_DIR_VAR: &str = "DXFEED_GRAAL_SDK_DIR";
/// Overrides the expected archive digest; only for testing the digest check.
const SHA256_OVERRIDE_VAR: &str = "DXFEED_GRAAL_SDK_SHA256";

/// SHA-256 of every `*.h` at the SDK root (see `headers_digest`). The committed bindings were generated from these
/// headers, so an SDK with other headers must not be linked.
const HEADERS_SHA256: &str = "159f0faf6106f181553fe406ae8e93fb65e4464d86e2c4e9a9b3400f26954835";

/// Supported targets: (`target_os`, `target_arch`, `target_env`, archive platform, archive SHA-256).
const PLATFORMS: &[(&str, &str, &str, &str, &str)] = &[
    (
        "windows",
        "x86_64",
        "msvc",
        "amd64-windows",
        "676ef7025ad3f33affe86d2eccb3c386d452b48c4c1b800806014891804bfd8c",
    ),
    (
        "linux",
        "x86_64",
        "gnu",
        "amd64-linux",
        "c39c5ed4eb2703f5136062405b1675d832ae181b4e42aa2e05bcc224b813fbcb",
    ),
    (
        "linux",
        "aarch64",
        "gnu",
        "aarch64-linux",
        "1d446776f158c537722414e6da4f5075f9b7cd7cbcad7469104ec2210518711c",
    ),
    (
        "macos",
        "x86_64",
        "",
        "x86_64-osx",
        "f61227a42b0d2c9406b823bfde80b5f0aebdf36f3885a86cacd7e5fbfd924d7d",
    ),
    (
        "macos",
        "aarch64",
        "",
        "aarch64-osx",
        "275bdb17bb941e14abbc5c7bde3430faa9ad6321c1663fa41d4177deb2f74380",
    ),
];

type Result<T> = std::result::Result<T, String>;

fn main() {
    if let Err(message) = run() {
        eprintln!("error: {message}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-env-changed=DOCS_RS");
    println!("cargo::rerun-if-env-changed={SDK_DIR_VAR}");
    println!("cargo::rerun-if-env-changed={SHA256_OVERRIDE_VAR}");
    if env::var_os("DOCS_RS").is_some() {
        return Ok(());
    }

    let target = Target::from_env()?;
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").ok_or("OUT_DIR is not set")?);

    let (lib_dir, include_dir) = match env::var_os(SDK_DIR_VAR) {
        Some(dir) => {
            let sdk = PathBuf::from(dir);
            println!("cargo::rerun-if-changed={}", sdk.display());
            check_sdk(&sdk, &target)?;
            (copy_libraries(&sdk, &target, &out_dir)?, sdk)
        }
        None => {
            let sdk = download_sdk(&target, &out_dir)?;
            check_sdk(&sdk, &target)?;
            (sdk.clone(), sdk)
        }
    };

    println!("cargo::rustc-link-search=native={}", lib_dir.display());
    println!("cargo::rustc-link-lib=dylib={LIB_NAME}");
    println!("cargo::metadata=lib_dir={}", lib_dir.display());
    println!("cargo::metadata=include_dir={}", include_dir.display());
    println!("cargo::metadata=sdk_version={SDK_VERSION}");
    Ok(())
}

struct Target {
    os: String,
    platform: &'static str,
    #[cfg_attr(not(feature = "download"), allow(dead_code))]
    sha256: &'static str,
}

impl Target {
    fn from_env() -> Result<Self> {
        let var = |name: &str| env::var(name).unwrap_or_default();
        let (os, arch, target_env) = (
            var("CARGO_CFG_TARGET_OS"),
            var("CARGO_CFG_TARGET_ARCH"),
            var("CARGO_CFG_TARGET_ENV"),
        );
        PLATFORMS
            .iter()
            .find(|(o, a, e, _, _)| *o == os && *a == arch && *e == target_env)
            .map(|&(_, _, _, platform, sha256)| Target { os, platform, sha256 })
            .ok_or_else(|| {
                format!(
                    "dxFeed Graal Native SDK {SDK_VERSION} has no build for target {}; supported targets: \
                     x86_64-pc-windows-msvc, x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu, x86_64-apple-darwin, \
                     aarch64-apple-darwin",
                    var("TARGET")
                )
            })
    }

    /// Files the linker and the loader need, all at the SDK root.
    fn libraries(&self) -> &'static [&'static str] {
        match self.os.as_str() {
            "windows" => &["DxFeedGraalNativeSdk.dll", "DxFeedGraalNativeSdk.lib"],
            "macos" => &["libDxFeedGraalNativeSdk.dylib"],
            _ => &["libDxFeedGraalNativeSdk.so"],
        }
    }
}

/// Checks that `sdk` holds the libraries and the headers of SDK 3.12.0.
fn check_sdk(sdk: &Path, target: &Target) -> Result<()> {
    for name in target.libraries().iter().chain(&["dxfg_api.h"]) {
        let path = sdk.join(name);
        if !path.is_file() {
            return Err(format!(
                "{} not found; {SDK_DIR_VAR} must name an unpacked dxFeed Graal Native SDK {SDK_VERSION} archive for {}",
                path.display(),
                target.platform
            ));
        }
    }
    let digest = headers_digest(sdk)?;
    if digest != HEADERS_SHA256 {
        return Err(format!(
            "the headers in {} differ from those of dxFeed Graal Native SDK {SDK_VERSION} (SHA-256 {digest}, expected \
             {HEADERS_SHA256}); the bindings of this crate fit only SDK {SDK_VERSION}",
            sdk.display()
        ));
    }
    Ok(())
}

/// SHA-256 over every `*.h` at the SDK root, sorted by name: the name, NUL, the contents with CRLF turned into LF
/// (Windows archives ship CRLF headers), NUL.
fn headers_digest(sdk: &Path) -> Result<String> {
    let mut names = Vec::new();
    for entry in fs::read_dir(sdk).map_err(|e| format!("cannot read {}: {e}", sdk.display()))? {
        let name = entry
            .map_err(|e| format!("cannot read {}: {e}", sdk.display()))?
            .file_name();
        if let Some(name) = name.to_str().filter(|n| n.ends_with(".h")) {
            names.push(name.to_owned());
        }
    }
    names.sort();
    let mut hasher = Sha256::new();
    for name in names {
        let path = sdk.join(&name);
        let contents = fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        hasher.update(name.as_bytes());
        hasher.update([0]);
        let mut rest = contents.as_slice();
        while let Some(pos) = rest.windows(2).position(|w| w == b"\r\n") {
            hasher.update(&rest[..pos]);
            rest = &rest[pos + 1..];
        }
        hasher.update(rest);
        hasher.update([0]);
    }
    Ok(hex(&hasher.finalize()))
}

/// Copies the libraries into `OUT_DIR/lib`, so cargo puts them on the loader path.
fn copy_libraries(sdk: &Path, target: &Target, out_dir: &Path) -> Result<PathBuf> {
    let lib_dir = out_dir.join("lib");
    fs::create_dir_all(&lib_dir).map_err(|e| format!("cannot create {}: {e}", lib_dir.display()))?;
    for name in target.libraries() {
        let (from, to) = (sdk.join(name), lib_dir.join(name));
        fs::copy(&from, &to).map_err(|e| format!("cannot copy {} to {}: {e}", from.display(), to.display()))?;
    }
    Ok(lib_dir)
}

fn archive_name(target: &Target) -> String {
    format!("graal-native-sdk-{SDK_VERSION}-{}", target.platform)
}

#[cfg(not(feature = "download"))]
fn download_sdk(target: &Target, _out_dir: &Path) -> Result<PathBuf> {
    Err(format!(
        "set {SDK_DIR_VAR} to an unpacked {}.zip, or enable the `download` feature",
        archive_name(target)
    ))
}

/// Downloads, verifies and unpacks the release archive into `OUT_DIR`, once.
#[cfg(feature = "download")]
fn download_sdk(target: &Target, out_dir: &Path) -> Result<PathBuf> {
    let name = archive_name(target);
    let sdk = out_dir.join(&name);
    if sdk.is_dir() {
        return Ok(sdk);
    }
    let url = format!("https://github.com/dxFeed/dxfeed-graal-native-sdk/releases/download/v{SDK_VERSION}/{name}.zip");
    let expected = env::var(SHA256_OVERRIDE_VAR).unwrap_or_else(|_| target.sha256.to_owned());
    let hint = format!("or set {SDK_DIR_VAR} to an unpacked copy of {url}");

    let part = out_dir.join(format!("{name}.zip.part"));
    let digest = download(&url, &part).map_err(|e| format!("cannot download {url}: {e}; check the network {hint}"))?;
    if digest != expected {
        let _ = fs::remove_file(&part);
        return Err(format!("{url} has SHA-256 {digest}, expected {expected}; {hint}"));
    }
    let archive = out_dir.join(format!("{name}.zip"));
    fs::rename(&part, &archive).map_err(|e| format!("cannot rename {}: {e}", part.display()))?;

    // Unpack next to the final directory and rename it into place, so an interrupted build never leaves a partial SDK.
    let unpacking = out_dir.join(format!("{name}.unpacking"));
    if unpacking.exists() {
        fs::remove_dir_all(&unpacking).map_err(|e| format!("cannot remove {}: {e}", unpacking.display()))?;
    }
    unpack(&archive, &unpacking).map_err(|e| format!("cannot unpack {}: {e}", archive.display()))?;
    fs::rename(&unpacking, &sdk).map_err(|e| format!("cannot rename {}: {e}", unpacking.display()))?;
    let _ = fs::remove_file(&archive);
    Ok(sdk)
}

/// Writes the body of `url` to `path` and returns its SHA-256. Uses the proxy environment variables and the
/// operating system's certificate store.
#[cfg(feature = "download")]
fn download(url: &str, path: &Path) -> std::result::Result<String, Box<dyn std::error::Error>> {
    use std::io::{self, Read, Write};
    use std::time::Duration;
    use ureq::tls::{RootCerts, TlsConfig};

    // ureq has no timeouts by default, so a stalled connection would hang the build forever. The global limit
    // assumes the SDK archive downloads within 10 minutes.
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .tls_config(TlsConfig::builder().root_certs(RootCerts::PlatformVerifier).build())
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_recv_response(Some(Duration::from_secs(60)))
        .timeout_global(Some(Duration::from_secs(10 * 60)))
        .build()
        .into();
    let mut response = agent.get(url).call()?;
    let mut body = response.body_mut().with_config().limit(256 * 1024 * 1024).reader();
    let mut file = io::BufWriter::new(fs::File::create(path)?);
    let mut hasher = Sha256::new();
    let mut buf = vec![0; 64 * 1024];
    loop {
        let n = body.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n])?;
    }
    file.flush()?;
    Ok(hex(&hasher.finalize()))
}

#[cfg(feature = "download")]
fn unpack(archive: &Path, dir: &Path) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut zip = zip::ZipArchive::new(fs::File::open(archive)?)?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        // enclosed_name() rejects absolute paths and `..`, so nothing lands outside `dir`.
        let Some(relative) = entry.enclosed_name() else {
            return Err(format!("unsafe path {:?} in the archive", entry.name()).into());
        };
        let path = dir.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(&path)?;
        } else {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            std::io::copy(&mut entry, &mut fs::File::create(&path)?)?;
        }
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
