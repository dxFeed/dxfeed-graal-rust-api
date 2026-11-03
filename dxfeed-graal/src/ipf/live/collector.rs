//! The collector of instrument profiles and its update listeners.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::ptr;
use std::sync::Arc;

use dxfeed_graal_sys as sys;

use crate::error::{Error, JavaException, Result, list_size};
use crate::executor::Executor;
use crate::handle::Handle;
use crate::ipf::InstrumentProfile;
use crate::ipf::profile::SysProfileList;
use crate::isolate::{self, call};
use crate::listener::{UpdateListener, UpdateListeners};
use crate::subscription::ListenerId;

/// A tag of a generation of profiles: [`update_instrument_profiles`] marks profiles with it, and
/// [`remove_generations`] removes the profiles that still have it, such as those a new data set did not update again.
///
/// It is a new Java object, which Java's collector keeps with the profiles; a tag equals only itself and its clones.
/// Keep a clone while its profiles may need removing: once all clones are dropped, they cannot be removed by their
/// generation.
///
/// [`update_instrument_profiles`]: InstrumentProfileCollector::update_instrument_profiles
/// [`remove_generations`]: InstrumentProfileCollector::remove_generations
#[derive(Clone)]
pub struct Generation {
    handle: Arc<Handle<sys::dxfg_v4_java_object_handler>>,
}

impl Generation {
    /// Creates a tag, different from every other one.
    pub fn new() -> Result<Generation> {
        // SAFETY: the out-pointer is valid for writes.
        let handle = Handle::create(|t, out| unsafe { sys::dxfg_v4_Object_new(t, out) })?;
        Ok(Generation {
            handle: Arc::new(handle),
        })
    }
}

impl PartialEq for Generation {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.handle, &other.handle)
    }
}

impl Eq for Generation {}

impl Hash for Generation {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.handle).hash(state);
    }
}

impl fmt::Debug for Generation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Generation({:p})", Arc::as_ptr(&self.handle))
    }
}

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
    /// [`REMOVED`](InstrumentProfile::REMOVED) (`updateInstrumentProfile`). The profile loses its [`Generation`], as
    /// with `update_instrument_profiles(.., None)`.
    #[doc(alias = "updateInstrumentProfile")]
    pub fn update_instrument_profile(&self, profile: &InstrumentProfile) -> Result<()> {
        let mut profile = profile.to_sys()?;
        let c = self.as_ptr();
        // SAFETY: the collector and the structure are live; the SDK only reads the structure.
        call(|t| unsafe { sys::dxfg_v4_InstrumentProfileCollector_updateInstrumentProfile(t, c, &mut profile.data) })
    }

    /// Updates several profiles at once, as [`update_instrument_profile`](Self::update_instrument_profile) does,
    /// and tags them with `generation` for [`remove_generations`](Self::remove_generations)
    /// (`updateInstrumentProfiles`).
    ///
    /// A profile that is updated again gets the new tag, `None` included; an update equal to the profile in the
    /// collector only changes its tag and does not reach the listeners.
    #[doc(alias = "updateInstrumentProfiles")]
    pub fn update_instrument_profiles(
        &self,
        profiles: &[InstrumentProfile],
        generation: Option<&Generation>,
    ) -> Result<()> {
        let profiles = SysProfileList::new(profiles)?;
        let generation = generation.map_or(ptr::null_mut(), |g| g.handle.as_ptr());
        let c = self.as_ptr();
        // SAFETY: the collector, the list, its structures and the tag are live; the SDK only reads them.
        call(|t| unsafe {
            sys::dxfg_v4_InstrumentProfileCollector_updateInstrumentProfiles(t, c, &profiles.list, generation)
        })
    }

    /// Removes the profiles tagged with one of `generations` and tells the listeners of the removals
    /// (`removeGenerations`). The profiles without a tag stay. Its time grows with the number of the profiles in the
    /// collector: call it sparingly.
    #[doc(alias = "removeGenerations")]
    pub fn remove_generations<'a>(&self, generations: impl IntoIterator<Item = &'a Generation>) -> Result<()> {
        let mut elements: Vec<_> = generations.into_iter().map(|g| g.handle.as_ptr()).collect();
        if elements.is_empty() {
            return Ok(());
        }
        let mut list = sys::dxfg_v4_java_object_handler_list {
            size: list_size(elements.len(), "generations")?,
            elements: elements.as_mut_ptr(),
        };
        let c = self.as_ptr();
        // SAFETY: the collector, the list and its tags are live; the SDK only reads them.
        call(|t| unsafe { sys::dxfg_v4_InstrumentProfileCollector_removeGenerations(t, c, &mut list) })
    }

    /// The executor that calls the update listeners (`getExecutor`); by default a single thread of the collector.
    #[doc(alias = "getExecutor")]
    pub fn get_executor(&self) -> Result<Executor> {
        let c = self.as_ptr();
        // SAFETY: the collector is live and the out-pointer is valid for writes.
        let handle =
            Handle::create(|t, out| unsafe { sys::dxfg_v4_InstrumentProfileCollector_getExecutor(t, c, out) })?;
        Ok(Executor::from_handle(handle))
    }

    /// Calls the update listeners on `executor` from now on (`setExecutor`), such as a thread pool or a
    /// [`ManualExecutor`](crate::ManualExecutor). The collector keeps the executor.
    #[doc(alias = "setExecutor")]
    pub fn set_executor(&self, executor: &Executor) -> Result<()> {
        let (c, e) = (self.as_ptr(), executor.handle.as_ptr());
        // SAFETY: the collector and the executor are live.
        call(|t| unsafe { sys::dxfg_v4_InstrumentProfileCollector_setExecutor(t, c, e) })
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
