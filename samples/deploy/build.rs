fn main() {
    // Copies the SDK library next to the executable and sets its rpath, so it runs outside cargo.
    dxfeed_graal_build::deploy().unwrap();
}
