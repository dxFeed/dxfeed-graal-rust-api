//! The collector of instrument profiles and its update listeners.

use std::ptr;
use std::sync::Arc;

use dxfeed_graal_sys as sys;

use crate::error::{Error, JavaException, Result};
use crate::handle::Handle;
use crate::ipf::InstrumentProfile;
use crate::isolate::{self, call};
use crate::listener::{UpdateListener, UpdateListeners};
use crate::subscription::ListenerId;

/// Collects instrument profiles and tells its listeners of the updates (`InstrumentProfileCollector`).
///
/// An [`InstrumentProfileConnection`](super::InstrumentProfileConnection) fills it from a file or a service; a
/// collector can also be updated directly. Profiles of the type [`REMOVED`](InstrumentProfile::REMOVED) remove
/// instruments. Clones share the collector.
#[derive(Clone)]
pub struct InstrumentProfileCollector {
    state: Arc<CollectorState>,
}

struct CollectorState {
    handle: Handle<sys::dxfg_v4_ipf_collector_t>,
    listeners: UpdateListeners,
}

impl Drop for CollectorState {
    fn drop(&mut self) {
        if isolate::exiting() {
            // No SDK calls from destructors while the process exits; the listeners' state is leaked.
            return;
        }
        let c = self.handle.as_ptr();
        self.listeners.remove_all("InstrumentProfileCollector", |l| {
            // SAFETY: the collector and the listener are live.
            call(|t| unsafe { sys::dxfg_v4_InstrumentProfileCollector_removeUpdateListener(t, c, l) })
        });
    }
}

impl InstrumentProfileCollector {
    /// Creates an empty collector.
    pub fn new() -> Result<Self> {
        // SAFETY: the out-pointer is valid for writes.
        let handle = Handle::create(|t, out| unsafe { sys::dxfg_v4_InstrumentProfileCollector_new(t, out) })?;
        Ok(InstrumentProfileCollector {
            state: Arc::new(CollectorState {
                handle,
                listeners: UpdateListeners::new("an update listener of this collector"),
            }),
        })
    }

    pub(crate) fn as_ptr(&self) -> *mut sys::dxfg_v4_ipf_collector_t {
        self.state.handle.as_ptr()
    }

    /// The time of the last update, in milliseconds since the Unix epoch (`getLastUpdateTime`).
    #[doc(alias = "getLastUpdateTime")]
    pub fn get_last_update_time(&self) -> Result<i64> {
        let mut time = 0;
        let c = self.as_ptr();
        // SAFETY: the collector is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_InstrumentProfileCollector_getLastUpdateTime(t, c, &mut time) })?;
        Ok(time)
    }

    /// Adds or replaces a profile, or removes its instrument if its type is
    /// [`REMOVED`](InstrumentProfile::REMOVED) (`updateInstrumentProfile`).
    #[doc(alias = "updateInstrumentProfile")]
    pub fn update_instrument_profile(&self, profile: &InstrumentProfile) -> Result<()> {
        let mut profile = profile.to_sys()?;
        let c = self.as_ptr();
        // SAFETY: the collector and the structure are live; the SDK only reads the structure.
        call(|t| unsafe { sys::dxfg_v4_InstrumentProfileCollector_updateInstrumentProfile(t, c, &mut profile.data) })
    }

    /// Updates several profiles at once, as [`update_instrument_profile`](Self::update_instrument_profile) does
    /// (`updateInstrumentProfiles`).
    #[doc(alias = "updateInstrumentProfiles")]
    pub fn update_instrument_profiles(&self, profiles: &[InstrumentProfile]) -> Result<()> {
        let mut profiles = profiles
            .iter()
            .map(InstrumentProfile::to_sys)
            .collect::<Result<Vec<_>>>()?;
        let mut elements: Vec<_> = profiles.iter_mut().map(|p| &raw mut p.data).collect();
        let list = sys::dxfg_v4_instrument_profile_data_list {
            size: elements.len() as i32,
            elements: elements.as_mut_ptr(),
        };
        let c = self.as_ptr();
        // SAFETY: the collector, the list and its structures are live; the SDK only reads them.
        call(|t| unsafe {
            sys::dxfg_v4_InstrumentProfileCollector_updateInstrumentProfiles(t, c, &list, ptr::null_mut())
        })
    }

    /// The profiles in the collector, with those of removed instruments for a while (`view`).
    pub fn view(&self) -> Result<Vec<InstrumentProfile>> {
        let c = self.as_ptr();
        // SAFETY: the collector is live and the out-pointer is valid for writes.
        let view = Handle::create(|t, out| unsafe { sys::dxfg_v4_InstrumentProfileCollector_view(t, c, out) })?;
        // SAFETY: the iterator is live.
        unsafe { collect(view.as_ptr()) }
    }

    /// Adds a listener of the updates and returns its id (`addUpdateListener`).
    ///
    /// The listener is called first with every profile in the collector, then with the profiles that changed,
    /// removed ones included. Calls of a listener do not overlap. It stays until it is removed or the last clone of
    /// the collector is dropped; a listener that holds a clone keeps the collector until it is removed.
    #[doc(alias = "addUpdateListener")]
    pub fn add_update_listener(
        &self,
        listener: impl FnMut(&[InstrumentProfile]) + Send + 'static,
    ) -> Result<ListenerId> {
        let c = self.as_ptr();
        self.state.listeners.add(UpdateListener::new(Box::new(listener))?, |l| {
            // SAFETY: the collector and the listener are live.
            call(|t| unsafe { sys::dxfg_v4_InstrumentProfileCollector_addUpdateListener(t, c, l) })
        })
    }

    /// Removes a listener, waiting for its running call unless called from it (`removeUpdateListener`).
    ///
    /// Returns [`Error::InvalidArgument`](crate::Error::InvalidArgument) if the listener is not in this collector.
    #[doc(alias = "removeUpdateListener")]
    pub fn remove_update_listener(&self, id: ListenerId) -> Result<()> {
        let c = self.as_ptr();
        self.state.listeners.remove(id, |l| {
            // SAFETY: the collector and the listener are live.
            call(|t| unsafe { sys::dxfg_v4_InstrumentProfileCollector_removeUpdateListener(t, c, l) })
        })
    }
}

/// Copies the profiles left in an SDK iterator.
///
/// # Safety
///
/// `iterator` must be live.
pub(crate) unsafe fn collect(iterator: *mut sys::dxfg_v4_iterable_ip_t) -> Result<Vec<InstrumentProfile>> {
    let mut profiles = Vec::new();
    loop {
        let mut has_next = 0;
        // SAFETY: the iterator is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_Iterable_InstrumentProfile_hasNext(t, iterator, &mut has_next) })?;
        if has_next == 0 {
            return Ok(profiles);
        }
        let mut data = ptr::null_mut();
        // SAFETY: as above.
        call(|t| unsafe { sys::dxfg_v4_Iterable_InstrumentProfile_next(t, iterator, &mut data) })?;
        if data.is_null() {
            return Err(Error::Java(JavaException::null_result()));
        }
        // SAFETY: the SDK filled the structure, which is released once after the copy.
        let profile = unsafe { InstrumentProfile::from_sys(data) };
        // SAFETY: as above; nothing uses it afterwards.
        crate::error_manager::report_release(
            "InstrumentProfileCollector",
            call(|t| unsafe { sys::dxfg_v4_instrument_profile_data_release(t, data) }),
        );
        profiles.push(profile?);
    }
}
