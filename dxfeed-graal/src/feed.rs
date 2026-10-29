//! The feed of an endpoint.

use std::sync::{Arc, OnceLock};

use dxfeed_graal_sys as sys;

use crate::endpoint::{DXEndpoint, EndpointState};
use crate::error::{Error, Result};
use crate::event::{Event, EventKind, EventType};
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

    /// Returns the feed of the default endpoint of the process, [`DXEndpoint::get_instance`] (`DXFeed.getInstance()`).
    ///
    /// The endpoint is configured on first use by the `dxfeed.properties` file and the system properties, such as
    /// `dxfeed.address`; closing it closes it for the whole process.
    #[doc(alias = "getInstance")]
    pub fn get_instance() -> Result<&'static DXFeed> {
        static INSTANCE: OnceLock<DXFeed> = OnceLock::new();
        if let Some(feed) = INSTANCE.get() {
            return Ok(feed);
        }
        let feed = DXEndpoint::get_instance()?.feed();
        Ok(INSTANCE.get_or_init(|| feed))
    }

    /// Creates a subscription to events of type `E`, [`Quote`](crate::event::Quote) or [`Trade`](crate::event::Trade), attached to
    /// this feed; its listeners receive `&QuoteRef` or `&TradeRef`.
    ///
    /// ```no_run
    /// # fn main() -> dxfeed_graal::Result<()> {
    /// use dxfeed_graal::DXEndpoint;
    /// use dxfeed_graal::event::Quote;
    ///
    /// let subscription = DXEndpoint::create()?.feed().create_subscription::<Quote>()?;
    /// subscription.add_event_listener(|quotes| quotes.for_each(|quote| println!("{}", quote.symbol())))?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// A subscription to several event types is created with
    /// [`create_subscription_with_types`](Self::create_subscription_with_types):
    ///
    /// ```compile_fail
    /// # fn main() -> dxfeed_graal::Result<()> {
    /// use dxfeed_graal::{DXEndpoint, Event};
    ///
    /// let subscription = DXEndpoint::create()?.feed().create_subscription::<Event>()?;
    /// # Ok(())
    /// # }
    /// ```
    #[doc(alias = "createSubscription")]
    pub fn create_subscription<E: EventKind>(&self) -> Result<DXFeedSubscription<E>> {
        let feed = self.endpoint.feed.as_ptr();
        // SAFETY: the feed is live and the out-pointer is valid for writes.
        let handle =
            Handle::create(|t, out| unsafe { sys::dxfg_v4_DXFeed_createSubscription(t, feed, E::TYPE.to_sys(), out) })?;
        Ok(DXFeedSubscription::new(handle, self.clone()))
    }

    /// Creates a subscription to events of `types`, attached to this feed; its listeners receive
    /// [`EventRef`](crate::EventRef).
    ///
    /// Returns [`Error::InvalidArgument`] if `types` is empty.
    #[doc(alias = "createSubscription")]
    pub fn create_subscription_with_types(&self, types: &[EventType]) -> Result<DXFeedSubscription<Event>> {
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
