fn main() {
    // Copies the SDK library next to the samples and sets their rpath, so they also run outside cargo.
    dxfeed_graal_build::deploy().unwrap();
}
