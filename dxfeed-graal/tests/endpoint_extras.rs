//! The properties, default instances, credentials and aggregation periods of endpoints, and authorization tokens.

use std::collections::HashSet;
use std::ptr;
use std::time::Duration;

use dxfeed_graal::event::Quote;
use dxfeed_graal::{
    AuthToken, DXEndpoint, DXFeedSubscription, DXPublisher, Error, EventKind, Role, TimePeriod, TimePeriodInfo,
};

#[test]
fn builders_tell_the_supported_properties() {
    let builder = DXEndpoint::new_builder();
    assert!(builder.supports_property(DXEndpoint::DXFEED_ADDRESS_PROPERTY).unwrap());
    assert!(builder.supports_property(DXEndpoint::NAME_PROPERTY).unwrap());
    assert!(!builder.supports_property("nope").unwrap());
    assert!(matches!(
        builder.supports_property("a\0b"),
        Err(Error::InvalidArgument(_))
    ));
    let publisher = DXEndpoint::new_builder().with_role(Role::Publisher);
    assert!(
        publisher
            .supports_property(DXEndpoint::DXPUBLISHER_ADDRESS_PROPERTY)
            .unwrap()
    );
}

#[test]
fn properties_come_from_maps_and_files() {
    let period = DXEndpoint::DXFEED_REQUESTED_AGGREGATION_PERIOD_PROPERTY;
    let endpoint = DXEndpoint::new_builder()
        .with_properties([(period, "2s")])
        .build()
        .unwrap();
    assert_eq!(
        endpoint.get_requested_aggregation_period().unwrap(),
        Some(TimePeriod::value_of_millis(2000))
    );

    let path = std::env::temp_dir().join(format!("dxfeed-graal-endpoint-{}.properties", std::process::id()));
    std::fs::write(&path, format!("# a comment\n{period}=3s\n")).unwrap();
    let file = path.to_str().unwrap();
    let endpoint = DXEndpoint::new_builder()
        .with_property(DXEndpoint::DXFEED_PROPERTIES_PROPERTY, file)
        .build();
    std::fs::remove_file(&path).unwrap();
    assert_eq!(
        endpoint.unwrap().get_requested_aggregation_period().unwrap(),
        Some(TimePeriod::value_of_millis(3000))
    );
    // As in Java, a missing file is skipped.
    let missing = DXEndpoint::new_builder()
        .with_property(DXEndpoint::DXFEED_PROPERTIES_PROPERTY, file)
        .build()
        .unwrap();
    assert_eq!(missing.get_requested_aggregation_period().unwrap(), None);
}

#[test]
fn default_instances_are_shared_per_role() {
    let publisher = DXEndpoint::get_instance_with_role(Role::Publisher).unwrap();
    assert!(ptr::eq(
        publisher,
        DXEndpoint::get_instance_with_role(Role::Publisher).unwrap()
    ));
    assert_eq!(publisher.get_role(), Role::Publisher);
    let hub = DXEndpoint::get_instance_with_role(Role::LocalHub).unwrap();
    assert!(!ptr::eq(publisher, hub));
    assert_eq!(hub.get_role(), Role::LocalHub);
    assert!(ptr::eq(
        DXPublisher::get_instance().unwrap(),
        DXPublisher::get_instance().unwrap()
    ));
    assert!(ptr::eq(
        DXEndpoint::get_instance_with_role(Role::Feed).unwrap(),
        DXEndpoint::get_instance().unwrap()
    ));
    let stream = DXEndpoint::get_instance_with_role(Role::StreamFeed).unwrap();
    assert!(ptr::eq(
        stream,
        DXEndpoint::get_instance_with_role(Role::StreamFeed).unwrap()
    ));
}

#[test]
fn endpoints_tell_their_event_types_and_take_credentials() {
    let endpoint = DXEndpoint::create().unwrap();
    let types: HashSet<_> = endpoint.get_event_types().unwrap().into_iter().collect();
    assert!(types.contains(&Quote::TYPE), "{types:?}");
    endpoint.user("demo").unwrap().password("demo").unwrap();
    assert!(matches!(endpoint.user("a\0b"), Err(Error::InvalidArgument(_))));
}

#[test]
fn requested_aggregation_period_round_trips() {
    let endpoint = DXEndpoint::create().unwrap();
    assert_eq!(endpoint.get_requested_aggregation_period().unwrap(), None);
    let second = TimePeriod::try_from(Duration::from_secs(1)).unwrap();
    endpoint.set_requested_aggregation_period(Some(second)).unwrap();
    assert_eq!(endpoint.get_requested_aggregation_period().unwrap(), Some(second));
    endpoint
        .set_requested_aggregation_period(Some(TimePeriod::ZERO))
        .unwrap();
    assert_eq!(
        endpoint.get_requested_aggregation_period().unwrap(),
        Some(TimePeriod::ZERO)
    );
    endpoint.set_requested_aggregation_period(None).unwrap();
    assert_eq!(endpoint.get_requested_aggregation_period().unwrap(), None);
    let too_long = TimePeriod::value_of_millis(DXFeedSubscription::AGGREGATION_PERIOD_MAX_VALUE.get_time() + 1);
    assert!(matches!(
        endpoint.set_requested_aggregation_period(Some(too_long)),
        Err(Error::InvalidArgument(_))
    ));
    assert_eq!(endpoint.get_aggregation_period_info().unwrap(), TimePeriodInfo::UNKNOWN);
}

#[test]
fn basic_tokens_match_java() {
    let token = AuthToken::create_basic_token("u", "p").unwrap();
    assert_eq!(token.get_scheme(), AuthToken::BASIC_SCHEME);
    assert_eq!(token.get_value(), "dTpw");
    assert_eq!(token.get_http_authorization(), "Basic dTpw");
    assert_eq!(token.to_string(), "Basic dTpw");
    assert_eq!((token.get_user(), token.get_password()), (Some("u"), Some("p")));
    assert_eq!(AuthToken::create_basic_token_from("u:p").unwrap(), token);
    assert_eq!(AuthToken::value_of("Basic dTpw").unwrap(), token);
    assert_eq!("Basic dTpw".parse::<AuthToken>().unwrap().get_user(), Some("u"));
    let colon = AuthToken::create_basic_token_from("a:b:c").unwrap();
    assert_eq!((colon.get_user(), colon.get_password()), (Some("a"), Some("b:c")));
    assert!(matches!(
        AuthToken::create_basic_token_from("no colon"),
        Err(Error::Java(_))
    ));
    assert_eq!(AuthToken::create_basic_token_or_none(None, Some("")).unwrap(), None);
    assert_eq!(AuthToken::create_basic_token_or_none(None, None).unwrap(), None);
    assert_eq!(AuthToken::create_basic_token_or_none(Some(""), Some("")).unwrap(), None);
    let custom = AuthToken::create_custom_token(AuthToken::BASIC_SCHEME, "dTpw").unwrap();
    assert_eq!((custom.get_user(), custom.get_password()), (None, None));
    assert_eq!(custom, token, "equal by the HTTP authorization, as in Java");
    assert_eq!(
        AuthToken::create_basic_token_or_none(Some("u"), None)
            .unwrap()
            .unwrap()
            .get_password(),
        Some("")
    );
    assert_eq!(format!("{token:?}"), r#"AuthToken { scheme: "Basic", .. }"#);
}

#[test]
fn bearer_and_custom_tokens_match_java() {
    let bearer = AuthToken::create_bearer_token("xyz").unwrap();
    assert_eq!(bearer.get_http_authorization(), "Bearer xyz");
    assert_eq!(
        (bearer.get_scheme(), bearer.get_value()),
        (AuthToken::BEARER_SCHEME, "xyz")
    );
    assert_eq!((bearer.get_user(), bearer.get_password()), (None, None));
    assert_eq!(AuthToken::value_of("Bearer xyz").unwrap(), bearer);
    assert!(matches!(AuthToken::create_bearer_token(""), Err(Error::Java(_))));
    assert_eq!(AuthToken::create_bearer_token_or_none(Some("")).unwrap(), None);
    assert_eq!(
        AuthToken::create_bearer_token_or_none(Some("xyz")).unwrap(),
        Some(bearer)
    );

    let custom = AuthToken::create_custom_token("Digest", "abc").unwrap();
    assert_eq!(custom.get_http_authorization(), "Digest abc");
    assert!(matches!(AuthToken::create_custom_token("", "abc"), Err(Error::Java(_))));
    assert!(matches!(AuthToken::value_of("nospace"), Err(Error::Java(_))));
    assert!(matches!(
        AuthToken::value_of("Basic a\0"),
        Err(Error::InvalidArgument(_))
    ));
}
