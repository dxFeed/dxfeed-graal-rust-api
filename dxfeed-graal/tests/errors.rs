//! Error reporting: Java exceptions, invalid arguments, system properties.

use std::error::Error as _;

use dxfeed_graal::{DXEndpoint, Error, Role, system};

#[test]
fn property_round_trip() {
    system::set_property("dxfeed.graal.rust.test", "1").unwrap();
    assert_eq!(
        system::get_property("dxfeed.graal.rust.test").unwrap().as_deref(),
        Some("1")
    );
    assert_eq!(system::get_property("dxfeed.graal.rust.unset").unwrap(), None);
}

#[test]
fn nul_in_property_is_invalid_argument() {
    assert!(matches!(
        system::set_property("a\0b", "1"),
        Err(Error::InvalidArgument(_))
    ));
    assert!(matches!(system::get_property("a\0b"), Err(Error::InvalidArgument(_))));
}

#[test]
fn nul_in_builder_property_is_invalid_argument() {
    let result = DXEndpoint::new_builder().with_property("a\0b", "1").build();
    assert!(matches!(result, Err(Error::InvalidArgument(_))));
}

#[test]
fn connect_to_invalid_address_is_java_exception() {
    let endpoint = DXEndpoint::create_with_role(Role::Feed).unwrap();
    let error = endpoint.connect("not-a-scheme://[").unwrap_err();
    let Error::Java(e) = &error else {
        panic!("expected a Java exception, got {error:?}");
    };
    assert!(!e.class_name().is_empty());
    assert!(!e.stack_trace().is_empty());
    let shown = error.to_string();
    assert!(shown.starts_with(e.class_name()), "{shown}");
    assert!(shown.contains(e.message()), "{shown}");
    if let Some(cause) = e.cause() {
        assert_eq!(error.source().unwrap().to_string(), cause.to_string());
    }
}
