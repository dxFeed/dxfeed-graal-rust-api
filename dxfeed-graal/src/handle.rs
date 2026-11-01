//! Owned SDK object handles.

use std::ptr::NonNull;
use std::sync::atomic::{AtomicUsize, Ordering};

use dxfeed_graal_sys as sys;

use crate::error::{Error, JavaException, Result};
use crate::isolate::{self, Thread, call};

/// An SDK handle structure whose first field is the `handler` of its Java object, or that `handler` itself.
///
/// # Safety
///
/// `handler` must return the `handler` field of a live structure of this type, or the structure itself for
/// `dxfg_v4_java_object_handler`.
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
    sys::dxfg_v4_on_demand_service_t,
    sys::dxfg_v4_price_level_service_t,
    sys::dxfg_v4_auth_order_source_t,
    sys::dxfg_v4_history_endpoint_t,
    sys::dxfg_v4_history_endpoint_builder_t,
    sys::dxfg_v4_endpoint_builder_t,
    sys::dxfg_v4_feed_t,
    sys::dxfg_v4_publisher_t,
    sys::dxfg_v4_executor_t,
    sys::dxfg_v4_subscription_t,
    sys::dxfg_v4_feed_event_listener_t,
    sys::dxfg_v4_endpoint_state_change_listener_t,
    sys::dxfg_v4_observable_subscription_t,
    sys::dxfg_v4_observable_subscription_change_listener_t,
    sys::dxfg_v4_instrument_profile_reader_t,
    sys::dxfg_v4_instrument_profile_custom_fields_t,
    sys::dxfg_v4_ipf_collector_t,
    sys::dxfg_v4_ipf_connection_t,
    sys::dxfg_v4_iterable_ip_t,
    sys::dxfg_v4_ipf_update_listener_t,
    sys::dxfg_v4_ipf_connection_state_change_listener_t,
    sys::dxfg_v4_schedule_t,
    sys::dxfg_v4_day_t,
    sys::dxfg_v4_session_t,
    sys::dxfg_v4_day_filter_t,
    sys::dxfg_v4_session_filter_t,
    sys::dxfg_v4_indexed_tx_model_t,
    sys::dxfg_v4_indexed_tx_model_builder_t,
    sys::dxfg_v4_time_series_tx_model_t,
    sys::dxfg_v4_time_series_tx_model_builder_t,
    sys::dxfg_v4_tx_model_listener_t,
    sys::dxfg_v4_indexed_event_tx_model_t,
    sys::dxfg_v4_indexed_event_tx_model_builder_t,
    sys::dxfg_v4_indexed_event_tx_model_listener_t,
    sys::dxfg_v4_subscription_controller_t,
    sys::dxfg_v4_time_period_t,
    sys::dxfg_v4_time_zone_t,
    sys::dxfg_v4_auth_token_t,
    sys::dxfg_v4_logging_listener_t,
    sys::dxfg_v4_input_stream_t,
    sys::dxfg_v4_price_increments_t,
    sys::dxfg_v4_additional_underlyings_t,
    sys::dxfg_v4_cfi_t,
    sys::dxfg_v4_cfi_value_t,
    sys::dxfg_v4_cfi_attribute_t
);

// SAFETY: a promise keeps its Java object handle in `handler`; the promises of an event and of a list start with a
// promise, so their handles are released through it.
unsafe impl SdkHandle for sys::dxfg_v4_promise_t {
    unsafe fn handler(this: *mut Self) -> *mut sys::dxfg_v4_java_object_handler {
        // SAFETY: the caller guarantees `this` is live.
        unsafe { &raw mut (*this).handler }
    }
}

// SAFETY: the handle of a plain Java object is its own `handler`.
unsafe impl SdkHandle for sys::dxfg_v4_java_object_handler {
    unsafe fn handler(this: *mut Self) -> *mut sys::dxfg_v4_java_object_handler {
        this
    }
}

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
    /// Returns [`Error::ShutDown`] after [`crate::system::shutdown`].
    pub(crate) fn create(f: impl FnOnce(Thread, *mut *mut T) -> i32) -> Result<Self> {
        Self::create_optional(f)?.ok_or_else(|| Error::Java(JavaException::null_result()))
    }

    /// Like [`create`](Self::create), for SDK functions that write NULL when there is no object: that gives `None`.
    pub(crate) fn create_optional(f: impl FnOnce(Thread, *mut *mut T) -> i32) -> Result<Option<Self>> {
        if isolate::shut_down() {
            return Err(Error::ShutDown);
        }
        let mut raw = std::ptr::null_mut();
        call(|t| f(t, &mut raw))?;
        // SAFETY: the SDK wrote a new handle or NULL, which is ours.
        Ok(unsafe { Self::from_raw(raw) })
    }

    /// Takes ownership of a handle that the SDK returned, such as an element of a list; NULL gives `None`.
    ///
    /// # Safety
    ///
    /// `raw` must be NULL or a live handle that the caller owns and releases nowhere else.
    pub(crate) unsafe fn from_raw(raw: *mut T) -> Option<Self> {
        let handle = NonNull::new(raw)?;
        LIVE.fetch_add(1, Ordering::SeqCst);
        Some(Handle(handle))
    }

    /// The raw pointer, valid while `self` lives.
    pub(crate) fn as_ptr(&self) -> *mut T {
        self.0.as_ptr()
    }

    /// Another handle of the same Java object, released apart from this one (`JavaObjectHandler_clone`).
    pub(crate) fn try_clone(&self) -> Result<Self> {
        // SAFETY: the handle is live.
        unsafe { Self::clone_raw(self.as_ptr()) }
    }

    /// A new handle of the Java object of the SDK handle `raw`, which stays with its owner.
    ///
    /// # Safety
    ///
    /// `raw` must be a live handle of type `T`.
    pub(crate) unsafe fn clone_raw(raw: *mut T) -> Result<Self> {
        // SAFETY: the caller guarantees that the handle is live, and the out-pointer is valid for writes. A typed
        // handle consists of the handler only, so the clone is a handle of the same type.
        Self::create(|t, out| unsafe { sys::dxfg_v4_JavaObjectHandler_clone(t, T::handler(raw), out.cast()) })
    }

    /// Releases the handle now and reports whether the SDK released it.
    pub(crate) fn release(self) -> Result<()> {
        let this = std::mem::ManuallyDrop::new(self);
        this.release_now()
    }

    fn release_now(&self) -> Result<()> {
        LIVE.fetch_sub(1, Ordering::SeqCst);
        if isolate::exiting() {
            // The process is exiting: leak the handle rather than call the SDK from an exit path.
            return Ok(());
        }
        let handler = self.as_ptr();
        // SAFETY: the handle is live and released once.
        call(|t| unsafe { sys::dxfg_v4_JavaObjectHandler_release(t, T::handler(handler)) })
    }
}

impl<T: SdkHandle> Drop for Handle<T> {
    fn drop(&mut self) {
        // Errors are cleared by `call`; a failed release leaks the Java object.
        crate::error_manager::report_release("JavaObjectHandler", self.release_now());
    }
}
