//! The version, the advanced revision and the features of the loaded SDK.

use dxfeed_graal::{Error, system};

#[test]
fn the_version_is_the_one_of_the_bindings() {
    assert_eq!(system::get_version().unwrap(), dxfeed_graal_sys::SDK_VERSION);
}

#[test]
fn the_advanced_revision_is_the_one_of_the_headers() {
    assert_eq!(system::get_advanced_revision().unwrap(), system::ADVANCED_REVISION);
    const { assert!(system::ADVANCED_REVISION >= 1) };
}

#[test]
fn every_feature_of_the_headers_is_supported() {
    for name in [
        system::feature::EVENT_QUEUE,
        system::feature::SHARED_EVENT_QUEUE,
        system::feature::EVENT_QUEUE_READY_SIGNAL,
        system::feature::FEED_FILTER,
        system::feature::TIME_SERIES_TX_MODEL_EVENT_PEEK,
        system::feature::MESSAGE_CONNECTORS,
        system::feature::CONNECTION_ERRORS,
    ] {
        assert!(system::is_feature_supported(name).unwrap(), "{name}");
    }
}

#[test]
fn the_feature_names_are_those_of_the_headers() {
    let name = |macro_value: &[u8]| {
        std::str::from_utf8(macro_value.strip_suffix(b"\0").unwrap())
            .unwrap()
            .to_owned()
    };
    assert_eq!(
        system::feature::EVENT_QUEUE,
        name(dxfeed_graal_sys::DXFG_V4_FEATURE_EVENT_QUEUE)
    );
    assert_eq!(
        system::feature::SHARED_EVENT_QUEUE,
        name(dxfeed_graal_sys::DXFG_V4_FEATURE_SHARED_EVENT_QUEUE)
    );
    assert_eq!(
        system::feature::EVENT_QUEUE_READY_SIGNAL,
        name(dxfeed_graal_sys::DXFG_V4_FEATURE_EVENT_QUEUE_READY_SIGNAL)
    );
    assert_eq!(
        system::feature::FEED_FILTER,
        name(dxfeed_graal_sys::DXFG_V4_FEATURE_FEED_FILTER)
    );
    assert_eq!(
        system::feature::TIME_SERIES_TX_MODEL_EVENT_PEEK,
        name(dxfeed_graal_sys::DXFG_V4_FEATURE_TIME_SERIES_TX_MODEL_EVENT_PEEK)
    );
    assert_eq!(
        system::feature::MESSAGE_CONNECTORS,
        name(dxfeed_graal_sys::DXFG_V4_FEATURE_MESSAGE_CONNECTORS)
    );
    assert_eq!(
        system::feature::CONNECTION_ERRORS,
        name(dxfeed_graal_sys::DXFG_V4_FEATURE_CONNECTION_ERRORS)
    );
}

#[test]
fn an_unknown_feature_is_not_supported() {
    assert!(!system::is_feature_supported("no_such_feature").unwrap());
    assert!(!system::is_feature_supported("").unwrap());
}

#[test]
fn a_feature_name_with_a_nul_byte_is_invalid() {
    assert!(matches!(
        system::is_feature_supported("event\0queue"),
        Err(Error::InvalidArgument(_))
    ));
}
