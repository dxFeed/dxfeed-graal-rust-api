//! The default endpoint and feed of the process; in their own test binary, so that no other test shares them.

use dxfeed_graal::event::Quote;
use dxfeed_graal::{DXEndpoint, DXFeed, Role, system};

#[test]
fn default_endpoint_and_feed_are_shared() {
    // Without `dxfeed.address` the default endpoint is created but stays unconnected.
    assert_eq!(system::get_property("dxfeed.address").unwrap(), None);
    let feed = DXFeed::get_instance().unwrap();
    let endpoint = DXEndpoint::get_instance().unwrap();
    assert!(std::ptr::eq(feed, DXFeed::get_instance().unwrap()));
    assert!(std::ptr::eq(endpoint, DXEndpoint::get_instance().unwrap()));
    assert_eq!(endpoint.get_role(), Role::Feed);

    let subscription = feed.create_subscription::<Quote>().unwrap();
    subscription.add_event_listener(|_| {}).unwrap();
    subscription.add_symbols(["AAPL"]).unwrap();
}
