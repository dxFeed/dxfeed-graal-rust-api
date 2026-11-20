/// The variable that `dxfeed-graal-sys` fills with the root of the unpacked SDK archive (`links` of that crate).
const SDK_ROOT_VAR: &str = "DEP_DXFEEDGRAALNATIVESDK_INCLUDE_DIR";

fn main() {
    // Copies the SDK library next to the executable and sets its rpath, so the tools run outside cargo.
    dxfeed_graal_build::deploy().unwrap();

    // The NetTest tests read the kit that the SDK archive ships with its documentation.
    println!("cargo::rerun-if-env-changed={SDK_ROOT_VAR}");
    let kit = match std::env::var_os(SDK_ROOT_VAR) {
        Some(root) => std::path::Path::new(&root)
            .join("docs")
            .join("wrapper-guide")
            .join("nettest"),
        // No SDK location, as on docs.rs: the kit tests fail with this path in their message.
        None => std::path::PathBuf::from(format!("<{SDK_ROOT_VAR} is not set>")),
    };
    println!("cargo::rustc-env=NETTEST_KIT_DIR={}", kit.display());
}
