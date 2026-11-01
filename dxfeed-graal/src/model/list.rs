//! A live list of events of a model (`ObservableListModel`) and its change listeners.

use std::fmt;
use std::marker::PhantomData;

use dxfeed_graal_sys as sys;

use crate::error::{Error, JavaException, Result};
use crate::event::sdk_event::take_events;
use crate::event::{EventKind, Events};
use crate::handle::Handle;
use crate::isolate::{self, call};
use crate::listener::{ListListener, ListListeners};
use crate::subscription::ListenerId;

/// A live list of the events of a model, such as the buy orders of an
/// [`OrderBookModel`](super::OrderBookModel) (`ObservableListModel`).
///
/// The list changes as the model receives events; [`to_vec`](Self::to_vec) copies what it holds now, and its
/// listeners are called after each change. Once the model is closed or dropped, the list keeps its last events but no
/// longer changes, and its listeners are not called. Dropping the list removes and releases its listeners.
pub struct ObservableListModel<E: EventKind> {
    handle: Handle<sys::dxfg_v4_observable_list_model_t>,
    listeners: ListListeners,
    _event: PhantomData<fn() -> E>,
}

impl<E: EventKind> ObservableListModel<E> {
    pub(crate) fn new(handle: Handle<sys::dxfg_v4_observable_list_model_t>) -> Self {
        ObservableListModel {
            handle,
            listeners: ListListeners::new("a listener of this list"),
            _event: PhantomData,
        }
    }

    /// The events in the list now, in its order (`toArray`).
    #[doc(alias = "toArray")]
    pub fn to_vec(&self) -> Result<Vec<E>> {
        let l = self.handle.as_ptr();
        // SAFETY: the list is live and the out-pointer is valid for writes.
        let events = take_events(|t, out| unsafe { sys::dxfg_v4_ObservableListModel_toArray(t, l, out) })?;
        events.ok_or_else(|| Error::Java(JavaException::null_result()))
    }

    /// Adds a listener of the changes and returns its id (`addListener`).
    ///
    /// The listener gets all the events of the list after each change, not only the changed ones, on the executor
    /// of the model or on the thread that changes the model; its calls do not overlap, and a change it makes itself
    /// through the model does not call it again (read the list with [`to_vec`](Self::to_vec) then). The events are
    /// valid during the call only: copy what you keep with `to_owned()`. The listener of a list of a closed model is
    /// never called. A panic in `listener` is caught and goes to [`ErrorManager`](crate::ErrorManager).
    #[doc(alias = "addListener")]
    pub fn add_listener(&self, mut listener: impl FnMut(Events<'_, E>) + Send + 'static) -> Result<ListenerId> {
        let l = self.handle.as_ptr();
        let listener = ListListener::new(Box::new(move |events: Events<'_>| listener(events.typed())))?;
        self.listeners.add(listener, |h| {
            // SAFETY: the list and the listener are live.
            call(|t| unsafe { sys::dxfg_v4_ObservableListModel_addListener(t, l, h) })
        })
    }

    /// Removes a listener, waiting for its running call unless called from it (`removeListener`).
    ///
    /// Returns [`Error::InvalidArgument`] if the listener is not in this list.
    #[doc(alias = "removeListener")]
    pub fn remove_listener(&self, id: ListenerId) -> Result<()> {
        let l = self.handle.as_ptr();
        self.listeners.remove(id, |h| {
            // SAFETY: the list and the listener are live.
            call(|t| unsafe { sys::dxfg_v4_ObservableListModel_removeListener(t, l, h) })
        })
    }
}

impl<E: EventKind> Drop for ObservableListModel<E> {
    fn drop(&mut self) {
        if isolate::exiting() {
            // No SDK calls while the process exits; the listeners leak, as the SDK may still call their trampoline.
            return;
        }
        let l = self.handle.as_ptr();
        self.listeners.remove_all("ObservableListModel", |h| {
            // SAFETY: the list and the listener are live.
            call(|t| unsafe { sys::dxfg_v4_ObservableListModel_removeListener(t, l, h) })
        });
    }
}

impl<E: EventKind> fmt::Debug for ObservableListModel<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ObservableListModel")
            .field("event_type", &E::TYPE)
            .finish_non_exhaustive()
    }
}
