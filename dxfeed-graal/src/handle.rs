//! Owned SDK object handles (KTD7).

use std::ptr::NonNull;
use std::sync::atomic::{AtomicUsize, Ordering};

use dxfeed_graal_sys as sys;

use crate::error::{Error, JavaException, Result};
use crate::isolate::{self, Thread, call};

/// An SDK handle structure whose first field is the `handler` of its Java object.
///
/// # Safety
///
/// `handler` must return the `handler` field of a live structure of this type.
pub(crate) unsafe trait SdkHandle {
    /// # Safety
    ///
    /// `this` must point to a live structure of this type.
    unsafe fn handler(this: *mut Self) -> *mut sys::dxfg_v4_java_object_handler;
}

macro_rules! sdk_handle {
    ($($t:ty),*) => {$(
        // SAFETY: every listed SDK structure has a `handler` field.
        unsafe impl SdkHandle for $t {
            unsafe fn handler(this: *mut Self) -> *mut sys::dxfg_v4_java_object_handler {
                // SAFETY: the caller guarantees `this` is live.
                unsafe { &raw mut (*this).handler }
            }
        }
    )*};
}

sdk_handle!(
    sys::dxfg_v4_endpoint_t,
    sys::dxfg_v4_endpoint_builder_t,
    sys::dxfg_v4_feed_t,
    sys::dxfg_v4_publisher_t,
    sys::dxfg_v4_executor_t
);

static LIVE: AtomicUsize = AtomicUsize::new(0);

/// The number of SDK handles owned by Rust values right now; for tests.
#[doc(hidden)]
pub fn live_handles() -> usize {
    LIVE.load(Ordering::SeqCst)
}

/// An owned SDK handle, released on `Drop`.
pub(crate) struct Handle<T: SdkHandle>(NonNull<T>);

// SAFETY: the handle is a pointer to SDK memory usable from any attached thread, and the Java objects behind the
// handle types of this crate are thread-safe.
unsafe impl<T: SdkHandle> Send for Handle<T> {}
// SAFETY: as above.
unsafe impl<T: SdkHandle> Sync for Handle<T> {}

impl<T: SdkHandle> Handle<T> {
    /// Calls an SDK function that writes a new handle to its out-pointer and takes ownership of it.
    pub(crate) fn create(f: impl FnOnce(Thread, *mut *mut T) -> i32) -> Result<Self> {
        let mut raw = std::ptr::null_mut();
        call(|t| f(t, &mut raw))?;
        let null = || {
            Error::Java(JavaException::new(
                String::new(),
                "the SDK returned NULL".into(),
                String::new(),
                None,
            ))
        };
        let handle = NonNull::new(raw).ok_or_else(null)?;
        LIVE.fetch_add(1, Ordering::SeqCst);
        Ok(Handle(handle))
    }

    /// The raw pointer, valid while `self` lives.
    pub(crate) fn as_ptr(&self) -> *mut T {
        self.0.as_ptr()
    }
}

impl<T: SdkHandle> Drop for Handle<T> {
    fn drop(&mut self) {
        LIVE.fetch_sub(1, Ordering::SeqCst);
        if isolate::exiting() {
            // The process is exiting: leak the handle rather than call the SDK from an exit path.
            return;
        }
        // SAFETY: the handle is live and released once; errors are cleared by `call`.
        let _ = call(|t| unsafe { sys::dxfg_v4_JavaObjectHandler_release(t, T::handler(self.as_ptr())) });
    }
}
