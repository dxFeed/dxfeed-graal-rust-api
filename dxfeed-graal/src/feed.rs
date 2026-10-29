//! The feed of an endpoint.

use std::sync::Arc;

use dxfeed_graal_sys as sys;

use crate::endpoint::EndpointState;
use crate::error::{Error, Result};
use crate::event::EventType;
use crate::handle::Handle;
use crate::subscription::DXFeedSubscription;

/// The feed of a [`DXEndpoint`](crate::DXEndpoint): creates subscriptions to events (`DXFeed`).
///
/// It keeps its endpoint open, as do its subscriptions; cloning it is cheap.
#[derive(Clone)]
pub struct DXFeed {
    endpoint: Arc<EndpointState>,
}

impl DXFeed {
    pub(crate) fn new(endpoint: Arc<EndpointState>) -> Self {
        DXFeed { endpoint }
    }

    /// Creates a subscription to events of `types`, attached to this feed.
    ///
    /// Returns [`Error::InvalidArgument`] if `types` is empty.
    pub fn create_subscription(&self, types: &[EventType]) -> Result<DXFeedSubscription> {
        if types.is_empty() {
            return Err(Error::InvalidArgument(
                "a subscription needs at least one event type".into(),
            ));
        }
        let mut classes: Vec<sys::dxfg_event_clazz_t> = types.iter().map(|t| t.to_sys()).collect();
        let mut pointers: Vec<*mut sys::dxfg_event_clazz_t> = classes.iter_mut().map(|c| c as *mut _).collect();
        let mut list = sys::dxfg_v4_event_clazz_list {
            size: pointers.len() as i32,
            elements: pointers.as_mut_ptr(),
        };
        let feed = self.endpoint.feed.as_ptr();
        // SAFETY: the feed and the list are live; the SDK only reads the list. The out-pointer is valid for writes.
        let handle = Handle::create(|t, out| unsafe {
            sys::dxfg_v4_DXFeed_createSubscriptionWithEventTypes(t, feed, &mut list, out)
        })?;
        Ok(DXFeedSubscription::new(handle, self.clone()))
    }
}
