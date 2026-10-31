//! The history endpoint without an address and at an address where nothing listens; no network needed.

use dxfeed_graal::Error;
use dxfeed_graal::event::{Candle, TimeAndSale};
use dxfeed_graal::history::{Compression, Format, HistoryEndpoint};

/// An address where nothing listens.
const UNREACHABLE: &str = "http://127.0.0.1:1/candledata";

#[test]
fn an_endpoint_needs_an_address() {
    let builder = HistoryEndpoint::new_builder()
        .with_user_name("demo")
        .with_password("demo");
    assert!(matches!(builder.build(), Err(Error::InvalidArgument(_))));
}

#[test]
fn a_nul_byte_in_a_setting_is_an_error() {
    let builder = HistoryEndpoint::new_builder().with_address(UNREACHABLE);
    assert!(matches!(
        builder.clone().with_auth_token("a\0b").build(),
        Err(Error::InvalidArgument(_))
    ));
    assert!(matches!(
        builder.with_address("a\0b").build(),
        Err(Error::InvalidArgument(_))
    ));
}

#[test]
fn an_unreachable_address_is_a_java_error() {
    let builder = HistoryEndpoint::new_builder()
        .with_address(UNREACHABLE)
        .with_user_name("demo")
        .with_password("demo");
    let endpoint = builder.build().unwrap();
    assert_refused(endpoint.get_time_series::<Candle>("IBM{=h}", 0, 1));
    assert_refused(builder.build().unwrap().get_time_series::<TimeAndSale>("IBM", 0, 1));
}

/// Asserts that a request failed to connect, not on its arguments.
fn assert_refused<T: std::fmt::Debug>(result: dxfeed_graal::Result<T>) {
    match result {
        Err(Error::Java(e)) => assert_eq!(e.get_class_name(), "java.net.ConnectException"),
        other => panic!("expected ConnectException, got {other:?}"),
    }
}

#[test]
fn every_setting_is_accepted() {
    for (compression, format) in [(Compression::None, Format::Text), (Compression::Gzip, Format::Csv)] {
        let endpoint = HistoryEndpoint::new_builder()
            .with_address(UNREACHABLE)
            .with_auth_token("token")
            .with_compression(compression)
            .with_format(format)
            .build()
            .unwrap();
        assert_refused(endpoint.get_time_series::<Candle>("IBM{=h}", 0, 1));
    }
}

#[test]
fn the_debug_form_hides_the_secrets() {
    let builder = HistoryEndpoint::new_builder()
        .with_address("http://demo:secret-password@127.0.0.1:1/candledata?a=b@c")
        .with_user_name("demo")
        .with_password("secret-password")
        .with_auth_token("secret-token");
    let debug = format!("{builder:?}");
    assert!(debug.contains("http://***@127.0.0.1:1/candledata?a=b@c"), "{debug}");
    assert!(debug.contains("demo") && !debug.contains("secret"), "{debug}");
}
