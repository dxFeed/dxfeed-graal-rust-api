//! The publisher of an endpoint.

use std::sync::Arc;

use dxfeed_graal_sys as sys;

use crate::endpoint::EndpointState;
use crate::error::Result;
use crate::event::{Event, SysEvents};
use crate::isolate::call;

/// The publisher of a [`DXEndpoint`](crate::DXEndpoint): publishes events to its subscribers (`DXPublisher`).
///
/// It keeps its endpoint open; cloning it is cheap.
#[derive(Clone)]
pub struct DXPublisher {
    state: Arc<EndpointState>,
}

impl DXPublisher {
    pub(crate) fn new(state: Arc<EndpointState>) -> Self {
        DXPublisher { state }
    }

    /// Publishes `events`. The endpoint's own feed receives them for the symbols it is subscribed to; a connected
    /// publisher endpoint also sends them to remote subscribers. Events without subscribers are dropped.
    ///
    /// Fails with [`Error::InvalidArgument`](crate::Error::InvalidArgument), publishing nothing, when a string holds a
    /// NUL byte or an exchange code does not fit a Java `char`; with [`Error::Java`](crate::Error::Java) when the SDK
    /// refuses the events. Java refuses an [`OrderBase`](crate::event::OrderBase) and an order with a negative index,
    /// with `SNAPSHOT_END` or `SNAPSHOT_SNIP` and a non-zero index, with an undefined side unless it is empty, or with
    /// a market maker over 8 characters; of a market maker of 5 to 8 characters it keeps the last 4.
    #[doc(alias = "publishEvents")]
    pub fn publish_events(&self, events: &[Event]) -> Result<()> {
        let mut sys_events = SysEvents::new(events)?;
        let mut list = sys_events.list();
        let p = self.state.publisher.as_ptr();
        // SAFETY: the publisher and the list are live; the SDK only reads the list and the events.
        call(|t| unsafe { sys::dxfg_v4_DXPublisher_publishEvents(t, p, &mut list) })
    }
}
