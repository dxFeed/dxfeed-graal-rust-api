//! The feed of an endpoint.

use dxfeed_graal_sys as sys;

use crate::handle::Handle;

/// The feed of an [`Endpoint`](crate::Endpoint): creates subscriptions to events.
pub struct Feed {
    #[allow(dead_code)] // Used by subscriptions (DXFR-4).
    pub(crate) handle: Handle<sys::dxfg_v4_feed_t>,
}
