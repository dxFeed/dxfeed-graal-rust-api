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
        system::feature::RUNTIME_STATISTICS,
        system::feature::MESSAGE_CONNECTOR_IO_STATISTICS,
        system::feature::BUILD_INFO,
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
    assert_eq!(
        system::feature::RUNTIME_STATISTICS,
        name(dxfeed_graal_sys::DXFG_V4_FEATURE_RUNTIME_STATISTICS)
    );
    assert_eq!(
        system::feature::MESSAGE_CONNECTOR_IO_STATISTICS,
        name(dxfeed_graal_sys::DXFG_V4_FEATURE_MESSAGE_CONNECTOR_IO_STATISTICS)
    );
    assert_eq!(
        system::feature::BUILD_INFO,
        name(dxfeed_graal_sys::DXFG_V4_FEATURE_BUILD_INFO)
    );
    assert_eq!(
        system::build_info::QD_VERSION,
        name(dxfeed_graal_sys::DXFG_V4_BUILD_INFO_QD_VERSION)
    );
    assert_eq!(
        system::build_info::VENDOR,
        name(dxfeed_graal_sys::DXFG_V4_BUILD_INFO_VENDOR)
    );
    assert_eq!(
        system::build_info::MARCH,
        name(dxfeed_graal_sys::DXFG_V4_BUILD_INFO_MARCH)
    );
    assert_eq!(system::build_info::GC, name(dxfeed_graal_sys::DXFG_V4_BUILD_INFO_GC));
}

#[test]
fn the_build_info_names_the_qds_version_the_runtime_and_the_gc() {
    let value = |key| {
        system::get_build_info(key)
            .unwrap()
            .unwrap_or_else(|| panic!("no {key}"))
    };
    let qd_version = value(system::build_info::QD_VERSION);
    assert!(
        qd_version.starts_with("3.") && qd_version[2..].chars().all(|c| c.is_ascii_digit()),
        "{qd_version}"
    );
    for key in [
        system::build_info::VENDOR,
        system::build_info::MARCH,
        system::build_info::GC,
    ] {
        assert!(!value(key).is_empty(), "{key}");
    }
}

#[test]
fn an_unknown_build_info_key_has_no_value() {
    assert_eq!(system::get_build_info("no_such_key").unwrap(), None);
    assert_eq!(system::get_build_info("").unwrap(), None);
    assert!(matches!(
        system::get_build_info("qd\0version"),
        Err(Error::InvalidArgument(_))
    ));
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
