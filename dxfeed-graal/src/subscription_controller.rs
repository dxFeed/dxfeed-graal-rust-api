//! The controller of a subscription: the feeds it is attached to and how it delivers the events.

use std::fmt;
use std::ptr;
use std::sync::Mutex;

use dxfeed_graal_sys as sys;

use crate::error::Result;
use crate::executor::Executor;
use crate::feed::DXFeed;
use crate::handle::Handle;
use crate::isolate::call;
use crate::subscription::{check_aggregation_period, check_events_batch_limit};
use crate::time_period::TimePeriod;

/// The controller of a subscription (`SubscriptionController`): it attaches the subscription to feeds and detaches it,
/// and changes how the events are delivered. Experimental in Java and in the SDK.
///
/// It comes from [`DXFeedSubscription::get_subscription_controller`] or from a model, such as
/// [`IndexedEventTxModel::get_subscription_controller`], and borrows it.
///
/// [`DXFeedSubscription::get_subscription_controller`]: crate::DXFeedSubscription::get_subscription_controller
/// [`IndexedEventTxModel::get_subscription_controller`]: crate::model::IndexedEventTxModel::get_subscription_controller
pub struct SubscriptionController<'a> {
    handle: Handle<sys::dxfg_v4_subscription_controller_t>,
    /// The feeds of the subscription or model, which keeps the feeds it is attached to.
    feeds: &'a Mutex<Vec<DXFeed>>,
    /// The subscription, if the controller is that of a subscription: a closed one keeps no feed.
    subscription: Option<&'a Handle<sys::dxfg_v4_subscription_t>>,
}

impl<'a> SubscriptionController<'a> {
    pub(crate) fn new(
        handle: Handle<sys::dxfg_v4_subscription_controller_t>,
        feeds: &'a Mutex<Vec<DXFeed>>,
        subscription: Option<&'a Handle<sys::dxfg_v4_subscription_t>>,
    ) -> Self {
        SubscriptionController {
            handle,
            feeds,
            subscription,
        }
    }

    /// Attaches the subscription to the feed; attaching it again, or a closed subscription, does nothing (`attach`).
    pub fn attach(&self, feed: &DXFeed) -> Result<()> {
        let c = self.handle.as_ptr();
        attach_feed(self.feeds, feed, |f| {
            // SAFETY: the controller and the feed are live.
            call(|t| unsafe { sys::dxfg_v4_SubscriptionController_attach(t, c, f) })?;
            let Some(subscription) = self.subscription else {
                return Ok(true);
            };
            let (s, mut closed) = (subscription.as_ptr(), 0);
            // SAFETY: the subscription is live and the out-pointer is valid for writes.
            call(|t| unsafe { sys::dxfg_v4_DXFeedSubscription_isClosed(t, s, &mut closed) })?;
            Ok(closed == 0)
        })
    }

    /// Detaches the subscription from the feed (`detach`).
    pub fn detach(&self, feed: &DXFeed) -> Result<()> {
        let c = self.handle.as_ptr();
        detach_feed(self.feeds, feed, |f| {
            // SAFETY: the controller and the feed are live.
            call(|t| unsafe { sys::dxfg_v4_SubscriptionController_detach(t, c, f) })
        })
    }

    /// The most events of one call of a listener (`getEventsBatchLimit`), see
    /// [`DXFeedSubscription::get_events_batch_limit`](crate::DXFeedSubscription::get_events_batch_limit).
    #[doc(alias = "getEventsBatchLimit")]
    pub fn get_events_batch_limit(&self) -> Result<i32> {
        let mut limit = 0;
        let c = self.handle.as_ptr();
        // SAFETY: the controller is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_SubscriptionController_getEventsBatchLimit(t, c, &mut limit) })?;
        Ok(limit)
    }

    /// Changes the most events of one call of a listener (`setEventsBatchLimit`); returns
    /// [`Error::InvalidArgument`](crate::Error::InvalidArgument) for a negative limit.
    #[doc(alias = "setEventsBatchLimit")]
    pub fn set_events_batch_limit(&self, limit: i32) -> Result<()> {
        check_events_batch_limit(limit)?;
        let c = self.handle.as_ptr();
        // SAFETY: the controller is live.
        call(|t| unsafe { sys::dxfg_v4_SubscriptionController_setEventsBatchLimit(t, c, limit) })
    }

    /// The executor that calls the listeners; `None` if it is the executor of the feed (`getExecutor`).
    #[doc(alias = "getExecutor")]
    pub fn get_executor(&self) -> Result<Option<Executor>> {
        let c = self.handle.as_ptr();
        // SAFETY: the controller is live and the out-pointer is valid for writes.
        let handle =
            Handle::create_optional(|t, out| unsafe { sys::dxfg_v4_SubscriptionController_getExecutor(t, c, out) })?;
        Ok(handle.map(Executor::from_handle))
    }

    /// Changes the executor that calls the listeners; `None` for the executor of the feed (`setExecutor`).
    #[doc(alias = "setExecutor")]
    pub fn set_executor(&self, executor: Option<&Executor>) -> Result<()> {
        let c = self.handle.as_ptr();
        let e = executor.map_or(ptr::null_mut(), |executor| executor.handle.as_ptr());
        // SAFETY: the controller and the executor are live.
        call(|t| unsafe { sys::dxfg_v4_SubscriptionController_setExecutor(t, c, e) })
    }

    /// How long the events are gathered before the listeners are called; `None` if no period was set
    /// (`getAggregationPeriod`).
    #[doc(alias = "getAggregationPeriod")]
    pub fn get_aggregation_period(&self) -> Result<Option<TimePeriod>> {
        let c = self.handle.as_ptr();
        // SAFETY: the controller is live and the out-pointer is valid for writes.
        let period = Handle::create_optional(|t, out| unsafe {
            sys::dxfg_v4_SubscriptionController_getAggregationPeriod(t, c, out)
        })?;
        period.as_ref().map(TimePeriod::from_sdk).transpose()
    }

    /// Changes the aggregation period; `None` clears it (`setAggregationPeriod`), see
    /// [`DXFeedSubscription::set_aggregation_period`](crate::DXFeedSubscription::set_aggregation_period).
    #[doc(alias = "setAggregationPeriod")]
    pub fn set_aggregation_period(&self, period: Option<TimePeriod>) -> Result<()> {
        check_aggregation_period(period)?;
        let period = period.map(TimePeriod::to_sdk).transpose()?;
        let c = self.handle.as_ptr();
        let p = period.as_ref().map_or(ptr::null_mut(), Handle::as_ptr);
        // SAFETY: the controller and the period, if any, are live.
        call(|t| unsafe { sys::dxfg_v4_SubscriptionController_setAggregationPeriod(t, c, p) })
    }
}

/// Attaches a subscription or model to `feed` with `sdk`, which returns whether to keep the feed, and keeps it in
/// `feeds`. The lock keeps the SDK call and the kept feeds in step with a concurrent detach.
pub(crate) fn attach_feed(
    feeds: &Mutex<Vec<DXFeed>>,
    feed: &DXFeed,
    sdk: impl FnOnce(*mut sys::dxfg_v4_feed_t) -> Result<bool>,
) -> Result<()> {
    let mut kept = crate::lock(feeds);
    if sdk(feed.as_ptr())? && !kept.iter().any(|kept| kept.same_as(feed)) {
        kept.push(feed.clone());
    }
    Ok(())
}

/// Detaches a subscription or model from `feed` with `sdk` and lets go of the feed in `feeds`.
pub(crate) fn detach_feed(
    feeds: &Mutex<Vec<DXFeed>>,
    feed: &DXFeed,
    sdk: impl FnOnce(*mut sys::dxfg_v4_feed_t) -> Result<()>,
) -> Result<()> {
    let mut kept = crate::lock(feeds);
    sdk(feed.as_ptr())?;
    kept.retain(|kept| !kept.same_as(feed));
    Ok(())
}

impl fmt::Debug for SubscriptionController<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SubscriptionController").finish_non_exhaustive()
    }
}
