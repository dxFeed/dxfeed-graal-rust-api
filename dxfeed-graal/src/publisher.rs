//! The publisher of an endpoint.

use dxfeed_graal_sys as sys;

use crate::error::Result;
use crate::event::{Event, SysEvents};
use crate::handle::Handle;
use crate::isolate::call;

/// The publisher of an [`Endpoint`](crate::Endpoint): publishes events to its subscribers.
pub struct Publisher {
    pub(crate) handle: Handle<sys::dxfg_v4_publisher_t>,
}

impl Publisher {
    /// Publishes `events`. The endpoint's own feed receives them for the symbols it is subscribed to; a connected
    /// publisher endpoint also sends them to remote subscribers. Events without subscribers are dropped.
    pub fn publish(&self, events: &[Event]) -> Result<()> {
        let mut sys_events = SysEvents::new(events)?;
        let mut list = sys_events.list();
        let p = self.handle.as_ptr();
        // SAFETY: the publisher and the list are live; the SDK only reads the list and the events.
        call(|t| unsafe { sys::dxfg_v4_DXPublisher_publishEvents(t, p, &mut list) })
    }
}
