//! Observable subscriptions and the listeners of their changes (`ObservableSubscription`,
//! `ObservableSubscriptionChangeListener`).

use std::ptr;
use std::sync::{Arc, Mutex};

use dxfeed_graal_sys as sys;

use crate::error::{Error, Result};
use crate::event::EventType;
use crate::handle::Handle;
use crate::isolate::call;
use crate::listener::ChangeListener;
use crate::publisher::DXPublisher;
use crate::subscription::ListenerId;
use crate::symbol::Symbol;

/// A listener of the symbols of a subscription (`ObservableSubscriptionChangeListener`).
///
/// A closure `FnMut(&[Symbol])` is a listener of the added symbols only, as a Java lambda is. The calls of a listener
/// do not overlap; they run on the thread that changes the subscription, and the first one on the thread that adds the
/// listener. A panic in a call is caught and printed to stderr; the listener stays active.
///
/// ```no_run
/// # fn main() -> dxfeed_graal::Result<()> {
/// use dxfeed_graal::event::Profile;
/// use dxfeed_graal::{DXEndpoint, ObservableSubscriptionChangeListener, Role, Symbol};
///
/// struct Printer;
///
/// impl ObservableSubscriptionChangeListener for Printer {
///     fn symbols_added(&mut self, symbols: &[Symbol]) {
///         println!("added {symbols:?}");
///     }
///
///     fn symbols_removed(&mut self, symbols: &[Symbol]) {
///         println!("removed {symbols:?}");
///     }
/// }
///
/// let endpoint = DXEndpoint::create_with_role(Role::Publisher)?;
/// let subscription = endpoint.publisher().subscription::<Profile>()?;
/// subscription.add_change_listener(Printer)?;
/// subscription.add_change_listener(|symbols: &[Symbol]| println!("added {symbols:?}"))?;
/// # Ok(())
/// # }
/// ```
pub trait ObservableSubscriptionChangeListener: Send + 'static {
    /// Called with the symbols added to the subscription; when the listener is added, with the symbols the subscription
    /// has, if any (`symbolsAdded`).
    #[doc(alias = "symbolsAdded")]
    fn symbols_added(&mut self, symbols: &[Symbol]);

    /// Called with the symbols removed from the subscription (`symbolsRemoved`); does nothing by default.
    #[doc(alias = "symbolsRemoved")]
    fn symbols_removed(&mut self, symbols: &[Symbol]) {
        let _ = symbols;
    }

    /// Called once when the subscription is closed or the listener is removed (`subscriptionClosed`); does nothing by
    /// default.
    #[doc(alias = "subscriptionClosed")]
    fn subscription_closed(&mut self) {}
}

impl<F: FnMut(&[Symbol]) + Send + 'static> ObservableSubscriptionChangeListener for F {
    fn symbols_added(&mut self, symbols: &[Symbol]) {
        self(symbols)
    }
}

/// The subscription of a [`DXPublisher`] to the events of one type, whose change listeners learn the symbols that the
/// subscribers of the publisher ask for (`ObservableSubscription`); created with [`DXPublisher::subscription`].
///
/// The subscription lives as long as the publisher's endpoint and closes with it. As in Java, its change listeners stay
/// until they are removed or the endpoint is closed, also when this object is dropped, so that
/// `publisher.subscription::<Profile>()?.add_change_listener(listener)?` keeps the listener; a listener that holds the
/// publisher keeps the endpoint open until it is closed. This object keeps the endpoint open too.
pub struct ObservableSubscription {
    handle: Handle<sys::dxfg_v4_observable_subscription_t>,
    // Shared by the publisher's endpoint, which keeps the listeners until it is closed.
    listeners: Arc<ChangeListeners>,
    // Dropped last: it may close the endpoint.
    _publisher: DXPublisher,
}

impl ObservableSubscription {
    pub(crate) fn new(
        handle: Handle<sys::dxfg_v4_observable_subscription_t>,
        listeners: Arc<ChangeListeners>,
        publisher: DXPublisher,
    ) -> Self {
        ObservableSubscription {
            handle,
            listeners,
            _publisher: publisher,
        }
    }

    /// Whether the subscription is closed, as it is once its endpoint is closed (`isClosed`).
    #[doc(alias = "isClosed")]
    pub fn is_closed(&self) -> Result<bool> {
        let s = self.handle.as_ptr();
        let mut closed = 0;
        // SAFETY: the subscription is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_ObservableSubscription_isClosed(t, s, &mut closed) })?;
        Ok(closed != 0)
    }

    /// The event types of the subscription: the one it was created for (`getEventTypes`).
    #[doc(alias = "getEventTypes")]
    pub fn event_types(&self) -> Result<Vec<EventType>> {
        let s = self.handle.as_ptr();
        let mut list = ptr::null_mut();
        // SAFETY: the subscription is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_ObservableSubscription_getEventTypes(t, s, &mut list) })?;
        // SAFETY: the SDK returned a live list of `size` live classes, or NULL.
        let types = unsafe { event_types(list) };
        // SAFETY: the list came from the SDK and is released once.
        call(|t| unsafe { sys::dxfg_v4_event_clazz_list_release(t, list) })?;
        Ok(types)
    }

    /// Whether the subscription is to the events of `event_type` (`containsEventType`).
    #[doc(alias = "containsEventType")]
    pub fn contains_event_type(&self, event_type: EventType) -> Result<bool> {
        let s = self.handle.as_ptr();
        let mut contains = 0;
        // SAFETY: the subscription is live and the out-pointer is valid for writes.
        call(|t| unsafe {
            sys::dxfg_v4_ObservableSubscription_containsEventType(t, s, event_type.to_sys(), &mut contains)
        })?;
        Ok(contains != 0)
    }

    /// Adds a change listener; before this returns, the listener is called with the symbols the subscription has
    /// (`addChangeListener`). Adding to a closed subscription does nothing.
    #[doc(alias = "addChangeListener")]
    pub fn add_change_listener(&self, listener: impl ObservableSubscriptionChangeListener) -> Result<ListenerId> {
        let s = self.handle.as_ptr();
        self.listeners.add(listener, |l| {
            // SAFETY: the subscription and the listener are live.
            call(|t| unsafe { sys::dxfg_v4_ObservableSubscription_addChangeListener(t, s, l) })
        })
    }

    /// Removes a change listener, which then gets its
    /// [`subscription_closed`](ObservableSubscriptionChangeListener::subscription_closed) call
    /// (`removeChangeListener`). Called outside the listener, it returns once no call of the listener runs; called
    /// from the listener, it returns at once and the listener is released in the background.
    ///
    /// Returns [`Error::InvalidArgument`] if the listener is not in this subscription.
    #[doc(alias = "removeChangeListener")]
    pub fn remove_change_listener(&self, id: ListenerId) -> Result<()> {
        let s = self.handle.as_ptr();
        self.listeners.remove(id, |l| {
            // SAFETY: the subscription and the listener are live.
            call(|t| unsafe { sys::dxfg_v4_ObservableSubscription_removeChangeListener(t, s, l) })
        })
    }
}

/// The change listeners of a subscription; dropping them releases those left.
pub(crate) struct ChangeListeners(Mutex<Vec<(ListenerId, ChangeListener)>>);

impl ChangeListeners {
    pub(crate) fn new() -> Self {
        ChangeListeners(Mutex::new(Vec::new()))
    }

    /// Creates the SDK listener of `listener` and adds it to the subscription with `add`.
    pub(crate) fn add(
        &self,
        listener: impl ObservableSubscriptionChangeListener,
        add: impl FnOnce(*mut sys::dxfg_v4_observable_subscription_change_listener_t) -> Result<()>,
    ) -> Result<ListenerId> {
        let listener = ChangeListener::new(Box::new(listener))?;
        if let Err(e) = add(listener.as_ptr()) {
            listener.release();
            return Err(e);
        }
        let id = ListenerId::next();
        crate::lock(&self.0).push((id, listener));
        Ok(id)
    }

    /// Removes the listener `id` from the subscription with `remove`, then releases it.
    pub(crate) fn remove(
        &self,
        id: ListenerId,
        remove: impl FnOnce(*mut sys::dxfg_v4_observable_subscription_change_listener_t) -> Result<()>,
    ) -> Result<()> {
        let listener = {
            let mut listeners = crate::lock(&self.0);
            let index = listeners.iter().position(|(i, _)| *i == id).ok_or_else(|| {
                Error::InvalidArgument(format!("{id:?} is not a change listener of this subscription"))
            })?;
            listeners.swap_remove(index).1
        };
        let removed = remove(listener.as_ptr());
        listener.release();
        removed
    }

    /// Removes every listener with `remove` and releases it, ignoring errors.
    pub(crate) fn remove_all(
        &self,
        remove: impl Fn(*mut sys::dxfg_v4_observable_subscription_change_listener_t) -> Result<()>,
    ) {
        let listeners = std::mem::take(&mut *crate::lock(&self.0));
        for (_, listener) in listeners {
            let _ = remove(listener.as_ptr());
            listener.release();
        }
    }
}

impl Drop for ChangeListeners {
    fn drop(&mut self) {
        // The listeners left belong to a closed subscription or endpoint, which no longer calls them.
        let listeners = std::mem::take(&mut *crate::lock(&self.0));
        for (_, listener) in listeners {
            listener.release();
        }
    }
}

/// The event types of an SDK list of classes, without those this crate does not know.
///
/// # Safety
///
/// `list` must be NULL or point to a live SDK list of `size` live classes.
unsafe fn event_types(list: *const sys::dxfg_v4_event_clazz_list) -> Vec<EventType> {
    // SAFETY: the caller guarantees that the list and its elements are live.
    unsafe {
        let Some(list) = list.as_ref() else {
            return Vec::new();
        };
        if list.elements.is_null() || list.size <= 0 {
            return Vec::new();
        }
        std::slice::from_raw_parts(list.elements, list.size as usize)
            .iter()
            .filter_map(|&class| class.as_ref())
            .filter_map(|&class| EventType::ALL.iter().copied().find(|t| t.to_sys() == class))
            .collect()
    }
}
