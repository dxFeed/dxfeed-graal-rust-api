//! Observable subscriptions and the listeners of their changes (`ObservableSubscription`,
//! `ObservableSubscriptionChangeListener`).

use std::ptr;
use std::sync::Arc;

use dxfeed_graal_sys as sys;

use crate::error::Result;
use crate::event::EventType;
use crate::handle::Handle;
use crate::isolate::call;
use crate::listener::{ChangeListener, ChangeListeners};
use crate::publisher::DXPublisher;
use crate::subscription::ListenerId;
use crate::symbol::Symbol;

/// A listener of the symbols of a subscription (`ObservableSubscriptionChangeListener`).
///
/// A closure `FnMut(&[Symbol])` is a listener of the added symbols only, as a Java lambda is. The calls of a listener
/// do not overlap; they run on the thread that changes the subscription, and the first one on the thread that adds the
/// listener. A panic in a call is caught and goes to [`ErrorManager`](crate::ErrorManager); the listener stays active.
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
/// let subscription = endpoint.get_publisher().get_subscription::<Profile>()?;
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
/// subscribers of the publisher ask for (`ObservableSubscription`); created with [`DXPublisher::get_subscription`].
///
/// The subscription lives as long as the publisher's endpoint and closes with it. As in Java, its change listeners stay
/// until they are removed or the endpoint is closed, also when this object is dropped, so that
/// `publisher.get_subscription::<Profile>()?.add_change_listener(listener)?` keeps the listener; a listener that holds the
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
    pub fn get_event_types(&self) -> Result<Vec<EventType>> {
        let s = self.handle.as_ptr();
        let mut list = ptr::null_mut();
        // SAFETY: the subscription is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_ObservableSubscription_getEventTypes(t, s, &mut list) })?;
        // SAFETY: the SDK returned a live list of `size` live classes, or NULL.
        let types = unsafe { event_types(list) };
        // SAFETY: the list came from the SDK and is released once.
        let released = call(|t| unsafe { sys::dxfg_v4_event_clazz_list_release(t, list) });
        crate::error_manager::report_release("ObservableSubscription", released);
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
        self.listeners.add(ChangeListener::new(Box::new(listener))?, |l| {
            // SAFETY: the subscription and the listener are live.
            call(|t| unsafe { sys::dxfg_v4_ObservableSubscription_addChangeListener(t, s, l) })
        })
    }

    /// Removes a change listener, which then gets its
    /// [`subscription_closed`](ObservableSubscriptionChangeListener::subscription_closed) call
    /// (`removeChangeListener`). Called outside the listener, it returns once no call of the listener runs; called
    /// from the listener, it returns at once and the listener is released in the background.
    ///
    /// Returns [`Error::InvalidArgument`](crate::Error::InvalidArgument) if the listener is not in this subscription.
    #[doc(alias = "removeChangeListener")]
    pub fn remove_change_listener(&self, id: ListenerId) -> Result<()> {
        let s = self.handle.as_ptr();
        self.listeners.remove(id, |l| {
            // SAFETY: the subscription and the listener are live.
            call(|t| unsafe { sys::dxfg_v4_ObservableSubscription_removeChangeListener(t, s, l) })
        })
    }
}

/// The members of Java's `ObservableSubscription` interface, which [`DXFeedSubscription`](crate::DXFeedSubscription),
/// [`DXFeedTimeSeriesSubscription`](crate::DXFeedTimeSeriesSubscription) and [`ObservableSubscription`] have: code
/// generic over this trait works with all of them.
///
/// The methods are also inherent methods of both types, so calling them needs no import of the trait. The trait is
/// sealed: only the types of this crate implement it.
pub trait Observable: sealed::Sealed {
    /// Whether the subscription is closed (`isClosed`).
    #[doc(alias = "isClosed")]
    fn is_closed(&self) -> Result<bool>;

    /// The event types of the subscription (`getEventTypes`).
    #[doc(alias = "getEventTypes")]
    fn get_event_types(&self) -> Result<Vec<EventType>>;

    /// Whether the subscription is to the events of `event_type` (`containsEventType`).
    #[doc(alias = "containsEventType")]
    fn contains_event_type(&self, event_type: EventType) -> Result<bool>;

    /// Adds a change listener; before this returns, the listener is called with the symbols the subscription has
    /// (`addChangeListener`).
    #[doc(alias = "addChangeListener")]
    fn add_change_listener(&self, listener: impl ObservableSubscriptionChangeListener) -> Result<ListenerId>
    where
        Self: Sized;

    /// Removes a change listener (`removeChangeListener`).
    #[doc(alias = "removeChangeListener")]
    fn remove_change_listener(&self, id: ListenerId) -> Result<()>;
}

pub(crate) mod sealed {
    pub trait Sealed {}
}

impl sealed::Sealed for ObservableSubscription {}

impl Observable for ObservableSubscription {
    fn is_closed(&self) -> Result<bool> {
        ObservableSubscription::is_closed(self)
    }

    fn get_event_types(&self) -> Result<Vec<EventType>> {
        ObservableSubscription::get_event_types(self)
    }

    fn contains_event_type(&self, event_type: EventType) -> Result<bool> {
        ObservableSubscription::contains_event_type(self, event_type)
    }

    fn add_change_listener(&self, listener: impl ObservableSubscriptionChangeListener) -> Result<ListenerId> {
        ObservableSubscription::add_change_listener(self, listener)
    }

    fn remove_change_listener(&self, id: ListenerId) -> Result<()> {
        ObservableSubscription::remove_change_listener(self, id)
    }
}

/// The event types of an SDK list of classes, without those this crate does not know.
///
/// # Safety
///
/// `list` must be NULL or point to a live SDK list of `size` live classes.
pub(crate) unsafe fn event_types(list: *const sys::dxfg_v4_event_clazz_list) -> Vec<EventType> {
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
