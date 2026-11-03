//! On-demand historical data replay (`com.dxfeed.ondemand`).
//!
//! An [`OnDemandService`] switches the feed of its endpoint to the replay of historical market data from a given time
//! with a given speed, and back. The replay needs an address with an `(ondemand:<address>)` component, which names a
//! provider of the historical data, and credentials for it.
//!
//! ```no_run
//! use dxfeed_graal::event::Quote;
//! use dxfeed_graal::ondemand::OnDemandService;
//! use dxfeed_graal::system;
//!
//! // The default on-demand endpoint is configured by the system properties as of its first use.
//! system::set_property("dxfeed.address", "(ondemand:demo.dxfeed.com:7680)")?;
//! system::set_property("dxfeed.user", "demo")?;
//! system::set_property("dxfeed.password", "demo")?;
//! let on_demand = OnDemandService::get_instance()?;
//! let subscription = on_demand.get_endpoint().get_feed().create_subscription::<Quote>()?;
//! subscription.add_event_listener(|quotes| {
//!     for quote in quotes {
//!         println!("{quote}");
//!     }
//! })?;
//! subscription.add_symbols(["ACN"])?;
//! on_demand.replay(1_273_175_268_000)?; // 2010-05-06 14:47:48 EST (19:47:48 UTC)
//! # Ok::<(), dxfeed_graal::Error>(())
//! ```
//!
//! A property change listener follows the replay: its `time`, `speed` and modes.

use std::fmt;
use std::sync::{Mutex, OnceLock};

use dxfeed_graal_sys as sys;

use crate::endpoint::DXEndpoint;
use crate::error::Result;
use crate::handle::Handle;
use crate::isolate::{self, call};
use crate::listener::{PropertyListener, PropertyListeners};
use crate::subscription::ListenerId;

/// The service of an endpoint that replays historical market data on demand (`OnDemandService`).
///
/// The replay of the service of [`get_instance`](Self::get_instance) runs on its own endpoint, which is connected to
/// no other feed; [`get_instance_for`](Self::get_instance_for) switches the feed of another endpoint to the replay and
/// back. Times are in milliseconds since the Unix epoch; a speed of 1 is the real-time speed.
pub struct OnDemandService {
    handle: Handle<sys::dxfg_v4_on_demand_service_t>,
    /// The endpoint of the service, kept open while the service lives.
    endpoint: DXEndpoint,
    /// The property change listeners added through this value.
    listeners: PropertyListeners,
}

impl Drop for OnDemandService {
    fn drop(&mut self) {
        if isolate::exiting() {
            // No SDK calls from destructors while the process exits; the listeners' state is leaked.
            return;
        }
        let s = self.handle.as_ptr();
        self.listeners.remove_all("OnDemandService", |l| {
            // SAFETY: the service and the listener are live.
            call(|t| unsafe { sys::dxfg_v4_OnDemandService_removePropertyChangeListener(t, s, l) })
        });
    }
}

/// The value of a property of an [`OnDemandService`] before or after a change, as a property change listener gets it.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum PropertyValue {
    /// No value (`null`).
    Null,
    /// A boolean: the properties `replaySupported`, `replay` and `clear`.
    Boolean(bool),
    /// A double: the property `speed`.
    Double(f64),
    /// A time in milliseconds since the Unix epoch: the property `time`.
    Time(i64),
    /// A value of a type that the crate does not know, of a property that a later SDK may add.
    Other,
}

impl PropertyValue {
    /// Converts a value of the SDK; a kind that the crate does not know is [`PropertyValue::Other`].
    pub(crate) fn from_sys(value: &sys::dxfg_v4_on_demand_property_value_t) -> Self {
        match value.kind {
            sys::DXFG_V4_ON_DEMAND_PROPERTY_VALUE_NULL => PropertyValue::Null,
            sys::DXFG_V4_ON_DEMAND_PROPERTY_VALUE_BOOLEAN => PropertyValue::Boolean(value.boolean_value != 0),
            sys::DXFG_V4_ON_DEMAND_PROPERTY_VALUE_DOUBLE => PropertyValue::Double(value.double_value),
            sys::DXFG_V4_ON_DEMAND_PROPERTY_VALUE_TIME => PropertyValue::Time(value.time_value),
            _ => PropertyValue::Other,
        }
    }
}

impl OnDemandService {
    /// The service of the default on-demand endpoint (`OnDemandService.getInstance()`). The endpoint has the
    /// [`Role::OnDemandFeed`](crate::Role::OnDemandFeed) role and is configured, as in Java, by the `dxfeed.properties`
    /// file and the system properties as of the first call. All callers share it, and it is never dropped: closing
    /// its endpoint closes it for the whole process.
    #[doc(alias = "getInstance")]
    pub fn get_instance() -> Result<&'static OnDemandService> {
        static INSTANCE: OnceLock<OnDemandService> = OnceLock::new();
        static INIT: Mutex<()> = Mutex::new(());
        crate::get_or_try_init(&INSTANCE, &INIT, || {
            // SAFETY: the out-pointer is valid for writes.
            let handle = Handle::create(|t, out| unsafe { sys::dxfg_v4_OnDemandService_getInstance(t, out) })?;
            let s = handle.as_ptr();
            // SAFETY: the service is live and the out-pointer is valid for writes.
            let endpoint = Handle::create(|t, out| unsafe { sys::dxfg_v4_OnDemandService_getEndpoint(t, s, out) })?;
            Ok(OnDemandService {
                handle,
                endpoint: DXEndpoint::new(endpoint, false)?,
                listeners: PropertyListeners::new("a property change listener of this service"),
            })
        })
    }

    /// The service of `endpoint`, which must have the [`Role::Feed`](crate::Role::Feed) or
    /// [`Role::OnDemandFeed`](crate::Role::OnDemandFeed) role (`OnDemandService.getInstance(DXEndpoint)`). Each
    /// endpoint has one service; the service keeps the endpoint open.
    #[doc(alias = "getInstance")]
    pub fn get_instance_for(endpoint: &DXEndpoint) -> Result<OnDemandService> {
        let e = endpoint.as_ptr();
        // SAFETY: the endpoint is live and the out-pointer is valid for writes.
        let handle = Handle::create(|t, out| unsafe { sys::dxfg_v4_OnDemandService_getInstanceByEndpoint(t, e, out) })?;
        Ok(OnDemandService {
            handle,
            endpoint: endpoint.clone(),
            listeners: PropertyListeners::new("a property change listener of this service"),
        })
    }

    /// The endpoint of the service (`getEndpoint`).
    #[doc(alias = "getEndpoint")]
    pub fn get_endpoint(&self) -> &DXEndpoint {
        &self.endpoint
    }

    /// Whether the replay is supported: the endpoint is connected to an address with an `(ondemand:<address>)`
    /// component (`isReplaySupported`). While it is not, the replay fails.
    #[doc(alias = "isReplaySupported")]
    pub fn is_replay_supported(&self) -> Result<bool> {
        self.flag(sys::dxfg_v4_OnDemandService_isReplaySupported)
    }

    /// Whether the service replays, after [`replay`](Self::replay) and before
    /// [`stop_and_resume`](Self::stop_and_resume) or [`stop_and_clear`](Self::stop_and_clear) (`isReplay`).
    #[doc(alias = "isReplay")]
    pub fn is_replay(&self) -> Result<bool> {
        self.flag(sys::dxfg_v4_OnDemandService_isReplay)
    }

    /// Whether the data is stopped and cleared, after [`stop_and_clear`](Self::stop_and_clear) and before the
    /// replay or [`stop_and_resume`](Self::stop_and_resume) (`isClear`).
    #[doc(alias = "isClear")]
    pub fn is_clear(&self) -> Result<bool> {
        self.flag(sys::dxfg_v4_OnDemandService_isClear)
    }

    /// The time being replayed, or the last one replayed; 0 if the replay never started (`getTime`).
    #[doc(alias = "getTime")]
    pub fn get_time(&self) -> Result<i64> {
        let mut time = 0;
        let s = self.handle.as_ptr();
        // SAFETY: the service is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_OnDemandService_getTime(t, s, &mut time) })?;
        Ok(time)
    }

    /// The replay speed; 0 while the service does not replay (`getSpeed`).
    #[doc(alias = "getSpeed")]
    pub fn get_speed(&self) -> Result<f64> {
        let mut speed = 0.0;
        let s = self.handle.as_ptr();
        // SAFETY: the service is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_OnDemandService_getSpeed(t, s, &mut speed) })?;
        Ok(speed)
    }

    /// Replays the data from `time` with the real-time speed (`replay`). The feed is disconnected from the other
    /// providers of data, buffers the historical data and replays it as if it came now. It fails while the replay is
    /// not supported (see [`is_replay_supported`](Self::is_replay_supported)).
    pub fn replay(&self, time: i64) -> Result<()> {
        let s = self.handle.as_ptr();
        // SAFETY: the service is live.
        call(|t| unsafe { sys::dxfg_v4_OnDemandService_replay(t, s, time) })
    }

    /// Replays the data from `time` with `speed`: more than 1 is faster than real time, less is slower and 0 is a
    /// pause (`replay`). A negative speed is an error.
    #[doc(alias = "replay")]
    pub fn replay_with_speed(&self, time: i64, speed: f64) -> Result<()> {
        let s = self.handle.as_ptr();
        // SAFETY: the service is live.
        call(|t| unsafe { sys::dxfg_v4_OnDemandService_replayWithSpeed(t, s, time, speed) })
    }

    /// Pauses the replay at the current time and keeps the data, as a speed of 0 does; does nothing while the
    /// service does not replay (`pause`).
    pub fn pause(&self) -> Result<()> {
        let s = self.handle.as_ptr();
        // SAFETY: the service is live.
        call(|t| unsafe { sys::dxfg_v4_OnDemandService_pause(t, s) })
    }

    /// Stops the replay and resumes the ordinary data feed; an on-demand endpoint has none, so for it this is
    /// [`stop_and_clear`](Self::stop_and_clear) (`stopAndResume`).
    #[doc(alias = "stopAndResume")]
    pub fn stop_and_resume(&self) -> Result<()> {
        let s = self.handle.as_ptr();
        // SAFETY: the service is live.
        call(|t| unsafe { sys::dxfg_v4_OnDemandService_stopAndResume(t, s) })
    }

    /// Stops the incoming data, of the replay or of the ordinary feed, and clears it (`stopAndClear`).
    #[doc(alias = "stopAndClear")]
    pub fn stop_and_clear(&self) -> Result<()> {
        let s = self.handle.as_ptr();
        // SAFETY: the service is live.
        call(|t| unsafe { sys::dxfg_v4_OnDemandService_stopAndClear(t, s) })
    }

    /// Changes the replay speed and goes on from the current time (`setSpeed`). A negative speed, or a speed other
    /// than 0 while the service does not replay, is an error.
    #[doc(alias = "setSpeed")]
    pub fn set_speed(&self, speed: f64) -> Result<()> {
        let s = self.handle.as_ptr();
        // SAFETY: the service is live.
        call(|t| unsafe { sys::dxfg_v4_OnDemandService_setSpeed(t, s, speed) })
    }

    /// Adds a listener of the property changes and returns its id (`addPropertyChangeListener`).
    ///
    /// The listener gets the name of the changed property and its old and new values, once per property:
    /// `replaySupported`, `replay` and `clear` as [`PropertyValue::Boolean`], `speed` as [`PropertyValue::Double`] and
    /// `time` as [`PropertyValue::Time`]. The name is `None` for a change of several properties at once, as a Java
    /// property change event without a name. As in Java, a change of `clear` is reported only when the same check of
    /// the service sees a change of `replay`, `speed` or `time` too, so it may be missed: [`is_clear`](Self::is_clear)
    /// tells the mode. The listener is called on a thread of the executor of the endpoint, not on the thread that
    /// changed the service, one change at a time. It stays until it is removed or this value is dropped;
    /// [`get_instance`](Self::get_instance) is never dropped.
    #[doc(alias = "addPropertyChangeListener")]
    pub fn add_property_change_listener(
        &self,
        listener: impl FnMut(Option<&str>, PropertyValue, PropertyValue) + Send + 'static,
    ) -> Result<ListenerId> {
        let s = self.handle.as_ptr();
        self.listeners.add(PropertyListener::new(Box::new(listener))?, |l| {
            // SAFETY: the service and the listener are live.
            call(|t| unsafe { sys::dxfg_v4_OnDemandService_addPropertyChangeListener(t, s, l) })
        })
    }

    /// Removes a listener, waiting for its running call unless called from it (`removePropertyChangeListener`).
    ///
    /// Returns [`Error::InvalidArgument`](crate::Error::InvalidArgument) if the listener was not added through this
    /// value.
    #[doc(alias = "removePropertyChangeListener")]
    pub fn remove_property_change_listener(&self, id: ListenerId) -> Result<()> {
        let s = self.handle.as_ptr();
        self.listeners.remove(id, |l| {
            // SAFETY: the service and the listener are live.
            call(|t| unsafe { sys::dxfg_v4_OnDemandService_removePropertyChangeListener(t, s, l) })
        })
    }

    fn flag(
        &self,
        f: unsafe extern "C" fn(
            *mut sys::graal_isolatethread_t,
            *mut sys::dxfg_v4_on_demand_service_t,
            *mut i32,
        ) -> i32,
    ) -> Result<bool> {
        let mut value = 0;
        let s = self.handle.as_ptr();
        // SAFETY: the service is live and the out-pointer is valid for writes.
        call(|t| unsafe { f(t, s, &mut value) })?;
        Ok(value != 0)
    }
}

impl fmt::Debug for OnDemandService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OnDemandService")
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(kind: i32) -> sys::dxfg_v4_on_demand_property_value_t {
        sys::dxfg_v4_on_demand_property_value_t {
            kind,
            boolean_value: 1,
            double_value: 2.5,
            time_value: 42,
        }
    }

    #[test]
    fn values_are_converted_by_kind() {
        assert_eq!(PropertyValue::from_sys(&value(0)), PropertyValue::Null);
        assert_eq!(PropertyValue::from_sys(&value(1)), PropertyValue::Boolean(true));
        assert_eq!(PropertyValue::from_sys(&value(2)), PropertyValue::Double(2.5));
        assert_eq!(PropertyValue::from_sys(&value(3)), PropertyValue::Time(42));
        assert_eq!(PropertyValue::from_sys(&value(4)), PropertyValue::Other);
        assert_eq!(PropertyValue::from_sys(&value(99)), PropertyValue::Other);
        assert_eq!(PropertyValue::from_sys(&value(-1)), PropertyValue::Other);
    }
}
