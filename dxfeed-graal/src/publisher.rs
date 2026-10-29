//! The publisher of an endpoint.

use dxfeed_graal_sys as sys;

use crate::handle::Handle;

/// The publisher of an [`Endpoint`](crate::Endpoint): publishes events to its subscribers.
pub struct Publisher {
    #[allow(dead_code)] // Used by publishing (DXFR-4).
    pub(crate) handle: Handle<sys::dxfg_v4_publisher_t>,
}
