//! Raw FFI bindings to the dxFeed Graal Native SDK C API (`dxfg_v4_` functions).
//!
//! The bindings are generated from the headers of SDK [`SDK_VERSION`] and committed, so building this crate needs
//! neither libclang nor the headers. Every function follows the SDK conventions: it takes the calling thread's
//! `graal_isolatethread_t`, returns `0` on success and `-1` on failure, and on failure leaves a Java exception for
//! `dxfg_v4_get_and_clear_thread_exception`. Use the `dxfeed-graal` crate for a safe API.
#![allow(non_upper_case_globals, non_camel_case_types, non_snake_case)]
#![allow(clippy::all, clippy::undocumented_unsafe_blocks)]
#![allow(rustdoc::broken_intra_doc_links, rustdoc::invalid_html_tags, rustdoc::bare_urls)]

#[cfg(not(target_pointer_width = "64"))]
compile_error!("dxfeed-graal-sys supports only 64-bit targets, as the dxFeed Graal Native SDK does");

/// The version of the dxFeed Graal Native SDK these bindings were generated from and the build script links.
pub const SDK_VERSION: &str = "3.12.0";

include!("bindings.rs");
