//! Handle ownership. One test only: the live-handle count is process-wide.

use std::cell::RefCell;

use dxfeed_graal::event::Quote;
use dxfeed_graal::{DXEndpoint, Role, live_handles};

thread_local! {
    static KEPT: RefCell<Option<DXEndpoint>> = const { RefCell::new(None) };
}

#[test]
fn handles_are_released() {
    let before = live_handles();
    {
        let endpoint = DXEndpoint::create_with_role(Role::LocalHub).unwrap();
        let _feed = endpoint.feed();
        let _publisher = endpoint.publisher();
        assert_eq!(live_handles(), before + 3);
    }
    assert_eq!(live_handles(), before);

    // A feed taken from a chain keeps the endpoint and its handles; the last handle dropped releases them.
    {
        let feed = DXEndpoint::create_with_role(Role::LocalHub).unwrap().feed();
        assert_eq!(live_handles(), before + 3);
        let subscription = feed.create_subscription::<Quote>().unwrap();
        drop(feed);
        assert_eq!(live_handles(), before + 4);
        drop(subscription);
    }
    assert_eq!(live_handles(), before);

    // An endpoint dropped by a thread-local destructor, possibly after the crate's own thread guard has run.
    std::thread::spawn(|| {
        KEPT.with(|_| ()); // registered before the crate's guard, so destroyed after it
        let endpoint = DXEndpoint::create_with_role(Role::LocalHub).unwrap();
        KEPT.with(|kept| *kept.borrow_mut() = Some(endpoint));
    })
    .join()
    .unwrap();
    assert_eq!(live_handles(), before);
    // The SDK still works after that.
    DXEndpoint::create_with_role(Role::LocalHub).unwrap();
}
