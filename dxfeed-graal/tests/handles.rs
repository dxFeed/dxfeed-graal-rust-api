//! Handle ownership. One test only: the live-handle count is process-wide.

use std::cell::RefCell;

use dxfeed_graal::{Endpoint, Role, live_handles};

thread_local! {
    static KEPT: RefCell<Option<Endpoint>> = const { RefCell::new(None) };
}

#[test]
fn handles_are_released() {
    let before = live_handles();
    {
        let endpoint = Endpoint::create(Role::LocalHub).unwrap();
        let _feed = endpoint.feed().unwrap();
        let _publisher = endpoint.publisher().unwrap();
        assert_eq!(live_handles(), before + 3);
    }
    assert_eq!(live_handles(), before);

    // An endpoint dropped by a thread-local destructor, possibly after the crate's own thread guard has run.
    std::thread::spawn(|| {
        KEPT.with(|_| ()); // registered before the crate's guard, so destroyed after it
        let endpoint = Endpoint::create(Role::LocalHub).unwrap();
        KEPT.with(|kept| *kept.borrow_mut() = Some(endpoint));
    })
    .join()
    .unwrap();
    assert_eq!(live_handles(), before);
    // The SDK still works after that.
    Endpoint::create(Role::LocalHub).unwrap();
}
