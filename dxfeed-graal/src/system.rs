//! Java system properties of the SDK.
//!
//! Most properties take effect only when set before the first endpoint is created.

use std::ptr;

use dxfeed_graal_sys as sys;

use crate::error::{Result, c_string};
use crate::isolate::{self, call};

/// Sets the Java system property `key` to `value` (`System.setProperty`).
pub fn set_property(key: &str, value: &str) -> Result<()> {
    let (key, value) = (c_string(key, "property key")?, c_string(value, "property value")?);
    // SAFETY: both strings are NUL-terminated and outlive the call.
    call(|t| unsafe { sys::dxfg_v4_system_set_property(t, key.as_ptr(), value.as_ptr()) })
}

/// Returns the Java system property `key`, or `None` if it is not set (`System.getProperty`).
pub fn get_property(key: &str) -> Result<Option<String>> {
    let key = c_string(key, "property key")?;
    let mut value = ptr::null();
    // SAFETY: the key is NUL-terminated and the out-pointer is valid for writes.
    call(|t| unsafe { sys::dxfg_v4_system_get_property(t, key.as_ptr(), &mut value) })?;
    if value.is_null() {
        return Ok(None);
    }
    // SAFETY: the SDK wrote a NUL-terminated string that is released right after copying.
    let copy = unsafe { isolate::string(value) };
    // SAFETY: `value` came from dxfg_v4_system_get_property and is released once.
    let _ = call(|t| unsafe { sys::dxfg_v4_system_release_property(t, value) });
    Ok(Some(copy))
}
