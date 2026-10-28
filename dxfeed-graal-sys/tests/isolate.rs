//! Checks that the SDK links and loads: cargo must put the library on the loader path without help.

use std::ptr;

use dxfeed_graal_sys::{graal_create_isolate, graal_isolate_t, graal_isolatethread_t, graal_tear_down_isolate};

#[test]
fn creates_and_tears_down_isolate() {
    let mut isolate: *mut graal_isolate_t = ptr::null_mut();
    let mut thread: *mut graal_isolatethread_t = ptr::null_mut();
    // SAFETY: null parameters select the defaults; the out-pointers are valid for writes.
    let created = unsafe { graal_create_isolate(ptr::null_mut(), &mut isolate, &mut thread) };
    assert_eq!(created, 0);
    assert!(!thread.is_null());
    // SAFETY: `thread` is the thread just attached by graal_create_isolate.
    assert_eq!(unsafe { graal_tear_down_isolate(thread) }, 0);
}
