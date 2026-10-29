//! Forwards the SDK location from `dxfeed-graal-sys` to the build scripts of crates that depend on `dxfeed-graal`.
//!
//! Cargo passes `cargo::metadata` only to direct dependents, so an application sees `DEP_DXFEED_GRAAL_LIB_DIR` (the
//! directory of the SDK runtime library) and `DEP_DXFEED_GRAAL_SDK_VERSION` only because this script re-emits them.
//! The `dxfeed-graal-build` crate reads them to deploy the library next to the application.

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    // The prefix comes from `links = "DxFeedGraalNativeSdk"` of dxfeed-graal-sys.
    for (var, key) in [
        ("DEP_DXFEEDGRAALNATIVESDK_LIB_DIR", "lib_dir"),
        ("DEP_DXFEEDGRAALNATIVESDK_SDK_VERSION", "sdk_version"),
    ] {
        println!("cargo::rerun-if-env-changed={var}");
        // Nothing is set on docs.rs, where the SDK is neither downloaded nor linked.
        if let Ok(value) = std::env::var(var) {
            println!("cargo::metadata={key}={value}");
        }
    }
}
