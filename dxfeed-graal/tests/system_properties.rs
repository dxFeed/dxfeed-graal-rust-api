//! The system properties file and the `DXFEED_*` environment variables, read when the isolate is created. The only
//! test of its binary: the variables must be set before the first SDK call of the process.

use dxfeed_graal::system;

#[test]
fn the_file_and_the_variables_become_system_properties() {
    let file = std::env::temp_dir().join(format!("dxfeed-graal-{}-system.properties", std::process::id()));
    std::fs::write(&file, "# test\nfile.only=from file\nboth=from file\n").unwrap();
    // SAFETY: no other thread of this process runs while the variables are set.
    unsafe {
        std::env::set_var("DXFEED_dxfeed.system.properties", &file);
        std::env::set_var("DXFEED_both", "from variable");
        std::env::set_var("DXFEED_variable.only", "from variable");
    }

    let property = |key| system::get_property(key).unwrap();
    assert_eq!(property("file.only").as_deref(), Some("from file"));
    assert_eq!(property("both").as_deref(), Some("from variable"));
    assert_eq!(property("variable.only").as_deref(), Some("from variable"));
    assert_eq!(
        property("dxfeed.system.properties").as_deref(),
        Some(file.to_str().unwrap())
    );
    let _ = std::fs::remove_file(&file);
}
