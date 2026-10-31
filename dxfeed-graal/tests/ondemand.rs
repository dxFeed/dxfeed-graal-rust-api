//! The on-demand service without a provider of historical data: its modes and the errors of the replay; no network
//! needed.

use dxfeed_graal::ondemand::OnDemandService;
use dxfeed_graal::{DXEndpoint, Role};

#[test]
fn the_default_service_has_its_own_endpoint() {
    let service = OnDemandService::get_instance().unwrap();
    assert!(std::ptr::eq(service, OnDemandService::get_instance().unwrap()));
    assert_eq!(service.endpoint().role(), Role::OnDemandFeed);
}

#[test]
fn a_service_without_a_provider_does_not_replay() {
    let endpoint = DXEndpoint::create().unwrap();
    let service = OnDemandService::get_instance_for(&endpoint).unwrap();
    assert_eq!(service.endpoint().role(), Role::Feed);
    assert!(!service.is_replay_supported().unwrap());
    assert!(!service.is_replay().unwrap());
    assert!(!service.is_clear().unwrap());
    assert_eq!(service.time().unwrap(), 0);
    assert_eq!(service.speed().unwrap(), 0.0);

    assert!(service.replay(1_273_175_268_000).is_err());
    assert!(service.replay_with_speed(1_273_175_268_000, 2.0).is_err());
    assert!(service.set_speed(1.0).is_err(), "no replay to speed up");
    service.set_speed(0.0).unwrap();
    service.pause().unwrap();
    assert!(!service.is_replay().unwrap());
}

#[test]
fn stop_and_clear_and_stop_and_resume_switch_the_clear_mode() {
    let endpoint = DXEndpoint::create().unwrap();
    let service = OnDemandService::get_instance_for(&endpoint).unwrap();
    service.stop_and_clear().unwrap();
    assert!(service.is_clear().unwrap());
    service.stop_and_resume().unwrap();
    assert!(!service.is_clear().unwrap());
}

#[test]
fn an_on_demand_address_supports_the_replay() {
    // Nothing listens there: the replay is supported, but no data comes.
    let endpoint = DXEndpoint::create_with_role(Role::OnDemandFeed).unwrap();
    endpoint.connect("(ondemand:127.0.0.1:1)").unwrap();
    let service = OnDemandService::get_instance_for(&endpoint).unwrap();
    assert!(service.is_replay_supported().unwrap());
    service.replay_with_speed(1_273_175_268_000, 2.0).unwrap();
    assert!(service.is_replay().unwrap());
    assert_eq!(service.speed().unwrap(), 2.0);
    service.pause().unwrap();
    assert_eq!(service.speed().unwrap(), 0.0);
    service.stop_and_resume().unwrap();
    assert!(!service.is_replay().unwrap());
    endpoint.close().unwrap();
}

#[test]
fn a_publisher_has_no_service() {
    let endpoint = DXEndpoint::create_with_role(Role::Publisher).unwrap();
    assert!(OnDemandService::get_instance_for(&endpoint).is_err());
}
