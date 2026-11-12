//! Gets dxFeed Graal Native SDK for the target and links it.
//!
//! The SDK comes from, in order: nowhere when `DOCS_RS` is set (no link directives); the unpacked archive in
//! `DXFEED_GRAAL_SDK_DIR`, whose library is copied into `OUT_DIR`; the GitHub release archive, verified against a
//! pinned SHA-256 and unpacked into `OUT_DIR` (feature `download`). With `DXFEED_GRAAL_SDK_CACHE_DIR` the verified
//! archive is kept in that directory and reused by later builds after its digest is checked again. Linking always
//! points at `OUT_DIR`, so cargo puts the library on the loader path for `cargo run` and `cargo test`.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
#[cfg(feature = "download")]
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

#[cfg(feature = "download")]
#[path = "build/retry.rs"]
mod retry;

const SDK_VERSION: &str = "3.16.0";
const LIB_NAME: &str = "DxFeedGraalNativeSdk";
const SDK_DIR_VAR: &str = "DXFEED_GRAAL_SDK_DIR";
/// An absolute directory where downloaded archives are kept across builds; unset or empty means no cache.
const CACHE_DIR_VAR: &str = "DXFEED_GRAAL_SDK_CACHE_DIR";
/// Overrides the expected archive digest; only for testing the digest check.
const SHA256_OVERRIDE_VAR: &str = "DXFEED_GRAAL_SDK_SHA256";

/// SHA-256 of every `*.h` at the SDK root (see `headers_digest`). The committed bindings were generated from these
/// headers, so an SDK with other headers must not be linked.
const HEADERS_SHA256: &str = "74138035fc63ba6c855ec65eb9bbe14e6aeeff61ac68561df914e43b77efa59f";

/// Supported targets: (`target_os`, `target_arch`, `target_env`, archive platform, archive SHA-256).
const PLATFORMS: &[(&str, &str, &str, &str, &str)] = &[
    (
        "windows",
        "x86_64",
        "msvc",
        "amd64-windows",
        "a1866cb8176ddc2428f882be50e1b877ba94f8a9f6646f5f8439e936ea4913a5",
    ),
    (
        "linux",
        "x86_64",
        "gnu",
        "amd64-linux",
        "b7eef6512bcc059f4c3629f288c7a8abfded0c7efa880aa63060ba5431484350",
    ),
    (
        "linux",
        "aarch64",
        "gnu",
        "aarch64-linux",
        "c9d3e2841309ba8de8cb8412dae03d263ccc0a299933bb4758a32abe81c4a077",
    ),
    (
        "macos",
        "x86_64",
        "",
        "x86_64-osx",
        "2a37f734742bd02139e9fec634b2356369c5b002233a33580c4f5c01c7529bfd",
    ),
    (
        "macos",
        "aarch64",
        "",
        "aarch64-osx",
        "516046ec33d3fdddf0cfd86968fe3b76b8d3030f3677bd04de43fee9e96bbc8d",
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
    println!("cargo::rerun-if-changed=build");
    println!("cargo::rerun-if-env-changed=DOCS_RS");
    println!("cargo::rerun-if-env-changed={SDK_DIR_VAR}");
    println!("cargo::rerun-if-env-changed={CACHE_DIR_VAR}");
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

/// Checks that `sdk` holds the libraries and the headers of the SDK.
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

/// Downloads, verifies and unpacks the release archive into `OUT_DIR`, once. With a cache directory the archive is
/// taken from it, or downloaded into it, and kept there.
#[cfg(feature = "download")]
fn download_sdk(target: &Target, out_dir: &Path) -> Result<PathBuf> {
    let name = archive_name(target);
    let sdk = out_dir.join(&name);
    if sdk.is_dir() {
        return Ok(sdk);
    }
    let source = Source {
        url: format!("https://github.com/dxFeed/dxfeed-graal-native-sdk/releases/download/v{SDK_VERSION}/{name}.zip"),
        expected: env::var(SHA256_OVERRIDE_VAR).unwrap_or_else(|_| target.sha256.to_owned()),
    };

    let cache_dir = cache_dir()?;
    let archive = match &cache_dir {
        Some(dir) => cached_archive(&source, dir, &name)?,
        None => {
            let archive = out_dir.join(format!("{name}.zip"));
            source.fetch(&out_dir.join(format!("{name}.zip.part")), &archive)?;
            archive
        }
    };

    // Unpack next to the final directory and rename it into place, so an interrupted build never leaves a partial SDK.
    let unpacking = out_dir.join(format!("{name}.unpacking"));
    if unpacking.exists() {
        fs::remove_dir_all(&unpacking).map_err(|e| format!("cannot remove {}: {e}", unpacking.display()))?;
    }
    unpack(&archive, &unpacking).map_err(|e| format!("cannot unpack {}: {e}", archive.display()))?;
    fs::rename(&unpacking, &sdk).map_err(|e| format!("cannot rename {}: {e}", unpacking.display()))?;
    if cache_dir.is_none() {
        let _ = fs::remove_file(&archive);
    }
    Ok(sdk)
}

/// The cache directory from `DXFEED_GRAAL_SDK_CACHE_DIR`. A relative path is rejected: build scripts run in the
/// package directory, so it would point somewhere the user did not mean.
#[cfg(feature = "download")]
fn cache_dir() -> Result<Option<PathBuf>> {
    let Some(dir) = env::var_os(CACHE_DIR_VAR).filter(|dir| !dir.is_empty()) else {
        return Ok(None);
    };
    let dir = PathBuf::from(dir);
    if dir.is_relative() {
        return Err(format!(
            "{CACHE_DIR_VAR} must be an absolute path, not {}",
            dir.display()
        ));
    }
    Ok(Some(dir))
}

/// Returns the verified archive in the cache, downloading it first when it is missing or does not match the digest.
/// Several builds may share the cache, so each downloads into its own `.part` file and renames it into place.
#[cfg(feature = "download")]
fn cached_archive(source: &Source, dir: &Path, name: &str) -> Result<PathBuf> {
    let archive = dir.join(format!("{name}.zip"));
    if archive.is_file() {
        match file_digest(&archive) {
            Ok(digest) if digest == source.expected => return Ok(archive),
            Ok(digest) => println!(
                "cargo::warning={} has SHA-256 {digest}, expected {}; downloading it again",
                archive.display(),
                source.expected
            ),
            Err(e) => println!(
                "cargo::warning=cannot read {}: {e}; downloading it again",
                archive.display()
            ),
        }
        let _ = fs::remove_file(&archive);
    }

    fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    remove_stale_parts(dir, name);
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let part = dir.join(format!("{name}.zip.{}-{nanos}.part", std::process::id()));
    match source.fetch(&part, &archive) {
        Ok(()) => Ok(archive),
        // Another build may have put a verified archive in place meanwhile, which also makes the rename fail on
        // Windows while that build reads it.
        Err(_) if file_digest(&archive).is_ok_and(|digest| digest == source.expected) => Ok(archive),
        Err(e) => Err(e),
    }
}

/// Removes `.part` files of downloads that were killed midway, which no later build reuses. A part file older than
/// an hour cannot belong to a running download: each attempt starts the file afresh and gives up after ten minutes.
#[cfg(feature = "download")]
fn remove_stale_parts(dir: &Path, name: &str) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    let prefix = format!("{name}.zip.");
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();
        let stale = || {
            entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|time| time.elapsed().ok())
                .is_some_and(|age| age > Duration::from_secs(60 * 60))
        };
        if file_name.starts_with(&prefix) && file_name.ends_with(".part") && stale() {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// The release archive and its expected SHA-256.
#[cfg(feature = "download")]
struct Source {
    url: String,
    expected: String,
}

#[cfg(feature = "download")]
impl Source {
    /// Downloads the archive to `part`, checks its digest and renames it to `archive`. Removes `part` on failure.
    fn fetch(&self, part: &Path, archive: &Path) -> Result<()> {
        let Self { url, expected } = self;
        let hint = format!("or set {SDK_DIR_VAR} to an unpacked copy of {url}");
        let result = match download(url, part) {
            Err((e, attempts)) => {
                let plural = if attempts == 1 { "" } else { "s" };
                Err(format!(
                    "cannot download {url} after {attempts} attempt{plural}: {e}; check the network {hint}"
                ))
            }
            Ok(digest) if digest != *expected => {
                Err(format!("{url} has SHA-256 {digest}, expected {expected}; {hint}"))
            }
            Ok(_) => fs::rename(part, archive).map_err(|e| format!("cannot rename {}: {e}", part.display())),
        };
        if result.is_err() {
            let _ = fs::remove_file(part);
        }
        result
    }
}

/// SHA-256 of the file at `path`.
#[cfg(feature = "download")]
fn file_digest(path: &Path) -> std::io::Result<String> {
    use std::io::Read;

    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

/// Writes the body of `url` to `path` and returns its SHA-256. Uses the proxy environment variables and the
/// operating system's certificate store. Tries again after a transient failure, such as a timeout or a 502, which
/// GitHub release downloads sometimes give; returns the last error and the number of attempts made.
#[cfg(feature = "download")]
fn download(url: &str, path: &Path) -> std::result::Result<String, (String, u32)> {
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
    retry::retry(
        retry::ATTEMPTS,
        || download_once(&agent, url, path),
        |attempt, error, delay| {
            println!(
                "cargo::warning=attempt {attempt} of {} to download {url} failed: {error}; retrying in {} s",
                retry::ATTEMPTS,
                delay.as_secs()
            );
            std::thread::sleep(delay);
        },
    )
}

/// One attempt of `download`. Starts `path` and the digest afresh, so a retry never keeps bytes of a failed attempt.
#[cfg(feature = "download")]
fn download_once(agent: &ureq::Agent, url: &str, path: &Path) -> std::result::Result<String, (retry::Failure, String)> {
    use std::io::{self, Read, Write};

    let network = |e: ureq::Error| (network_failure(&e), e.to_string());
    let local = |e: io::Error| {
        (
            retry::Failure::Permanent,
            format!("cannot write {}: {e}", path.display()),
        )
    };
    let mut response = agent.get(url).call().map_err(network)?;
    let mut body = response.body_mut().with_config().limit(256 * 1024 * 1024).reader();
    let mut file = io::BufWriter::new(fs::File::create(path).map_err(local)?);
    let mut hasher = Sha256::new();
    let mut buf = vec![0; 64 * 1024];
    loop {
        // The reader wraps ureq errors, such as a timeout, in io::Error; From unwraps them again.
        let n = body.read(&mut buf).map_err(|e| network(ureq::Error::from(e)))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n]).map_err(local)?;
    }
    file.flush().map_err(local)?;
    Ok(hex(&hasher.finalize()))
}

/// Timeouts, failed connections and HTTP 429 and 5xx are transient; other statuses, TLS and protocol errors are not.
#[cfg(feature = "download")]
fn network_failure(error: &ureq::Error) -> retry::Failure {
    match error {
        ureq::Error::StatusCode(status) => retry::status_failure(*status),
        ureq::Error::Io(e) => retry::io_failure(e.kind()),
        ureq::Error::Timeout(_) | ureq::Error::HostNotFound | ureq::Error::ConnectionFailed => {
            retry::Failure::Transient
        }
        _ => retry::Failure::Permanent,
    }
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
