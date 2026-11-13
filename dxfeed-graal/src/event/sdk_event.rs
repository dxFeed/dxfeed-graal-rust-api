//! Events the SDK returns outside listener calls, and the single symbols of the last-event requests.

use std::ptr::{self, NonNull};
use std::sync::atomic::{AtomicUsize, Ordering};

use dxfeed_graal_sys as sys;

use super::{Event, EventKind, EventType, LastingEvent, SubscriptionItem, sealed::Sealed};
use crate::error::{Error, JavaException, Result};
use crate::isolate::{self, Thread, call};

static LIVE: AtomicUsize = AtomicUsize::new(0);

/// The number of SDK events owned by this crate right now; for tests.
#[doc(hidden)]
pub fn live_events() -> usize {
    LIVE.load(Ordering::SeqCst)
}

/// An event the SDK returned, released on `Drop`.
struct SdkEvent(NonNull<sys::dxfg_event_type_t>);

impl SdkEvent {
    /// Copies the event as `E`; an event of another type is an error.
    fn to_owned<E: LastingEvent + EventKind>(&self) -> Result<E> {
        // SAFETY: the event is live while `self` lives, and the view is copied before `self` is dropped.
        let event = unsafe { <Event as Sealed>::item(self.0.as_ptr()) }.and_then(|e| e.to_owned());
        event.and_then(|e| E::try_from(e).ok()).ok_or_else(|| {
            Error::Java(JavaException::from_message(&format!(
                "the SDK returned an event that is not a {:?}",
                E::TYPE
            )))
        })
    }
}

impl Drop for SdkEvent {
    fn drop(&mut self) {
        LIVE.fetch_sub(1, Ordering::SeqCst);
        if isolate::exiting() {
            // The process is exiting: leak the event rather than call the SDK from an exit path.
            return;
        }
        let event = self.0.as_ptr();
        // SAFETY: the event came from the SDK and is released once. Errors are cleared by `call`; a failed release
        // leaks the event.
        crate::error_manager::report_release(
            "EventType",
            call(|t| unsafe { sys::dxfg_v4_EventType_release(t, event) }),
        );
    }
}

/// Calls an SDK function that writes a new event or NULL to its out-pointer, and copies the event as `E`.
pub(crate) fn take_event<E: LastingEvent + EventKind>(
    f: impl FnOnce(Thread, *mut *mut sys::dxfg_event_type_t) -> i32,
) -> Result<Option<E>> {
    let mut raw = ptr::null_mut();
    call(|t| f(t, &mut raw))?;
    let Some(event) = NonNull::new(raw) else {
        return Ok(None);
    };
    LIVE.fetch_add(1, Ordering::SeqCst);
    SdkEvent(event).to_owned().map(Some)
}

/// An event list the SDK returned, released on `Drop`.
struct SdkEventList(NonNull<sys::dxfg_event_type_list>, usize);

impl Drop for SdkEventList {
    fn drop(&mut self) {
        LIVE.fetch_sub(self.1, Ordering::SeqCst);
        if isolate::exiting() {
            return;
        }
        let list = self.0.as_ptr();
        // SAFETY: the list came from the SDK and is released once, with its events; a failed release leaks it.
        crate::error_manager::report_release(
            "EventType",
            call(|t| unsafe { sys::dxfg_v4_event_type_list_release(t, list) }),
        );
    }
}

/// Calls an SDK function that writes a new event list or NULL to its out-pointer, and copies the events as `E`.
pub(crate) fn take_events<E: EventKind + TryFrom<Event>>(
    f: impl FnOnce(Thread, *mut *mut sys::dxfg_event_type_list) -> i32,
) -> Result<Option<Vec<E>>> {
    let mut raw = ptr::null_mut();
    call(|t| f(t, &mut raw))?;
    let Some(list) = NonNull::new(raw) else {
        return Ok(None);
    };
    // SAFETY: the list is live while the guard lives.
    let len = usize::try_from(unsafe { list.as_ref() }.size).unwrap_or(0);
    LIVE.fetch_add(len, Ordering::SeqCst);
    let guard = SdkEventList(list, len);
    // SAFETY: the list is live while the guard lives, and the views are copied before it is dropped.
    let events = unsafe { super::Events::new(guard.0.as_ptr()) };
    events
        .map(|event| {
            event.to_owned().and_then(|e| E::try_from(e).ok()).ok_or_else(|| {
                Error::Java(JavaException::from_message(&format!(
                    "the SDK returned an event that is not a {:?}",
                    E::TYPE
                )))
            })
        })
        .collect::<Result<Vec<E>>>()
        .map(Some)
}

/// Whether the SDK finds the events of `event_type` only by candle symbols.
pub(crate) fn needs_candle_symbol(event_type: EventType) -> bool {
    matches!(event_type, EventType::Candle | EventType::DailyCandle)
}

/// An error for an event that the SDK returned but the crate cannot copy as the expected type.
fn unexpected_event() -> Error {
    Error::Java(JavaException::from_message(
        "the SDK returned an event of an unexpected type",
    ))
}

/// Copies the events of a list that the SDK gave to the caller, and releases the list; NULL is no events.
///
/// # Safety
///
/// `list` must be NULL or a live SDK event list that the caller owns and nothing else releases.
pub(crate) unsafe fn take_event_list<E: TryFrom<Event>>(list: *mut sys::dxfg_event_type_list) -> Result<Vec<E>> {
    let Some(list) = NonNull::new(list) else {
        return Ok(Vec::new());
    };
    // SAFETY: the caller guarantees that the list is live.
    let len = usize::try_from(unsafe { list.as_ref() }.size).unwrap_or(0);
    LIVE.fetch_add(len, Ordering::SeqCst);
    let guard = SdkEventList(list, len);
    // SAFETY: the list is live while the guard lives, and the views are copied before it is dropped.
    let events = unsafe { super::Events::new(guard.0.as_ptr()) };
    events
        .map(|event| {
            event
                .to_owned()
                .and_then(|e| E::try_from(e).ok())
                .ok_or_else(unexpected_event)
        })
        .collect()
}

/// Copies the events of a list that the SDK gave to the caller as the items of `E`, as an event stream copies the
/// events of a listener call, and releases the list; NULL is no events. Returns the copies and the number of the
/// events that the crate does not map ([`EventRef::Other`](super::EventRef::Other)), which it skips.
///
/// # Safety
///
/// `list` must be NULL or a live SDK event list that the caller owns and nothing else releases.
pub(crate) unsafe fn take_owned_list<E: SubscriptionItem>(list: *mut sys::dxfg_event_type_list) -> (Vec<E>, u64) {
    let Some(list) = NonNull::new(list) else {
        return (Vec::new(), 0);
    };
    // SAFETY: the caller guarantees that the list is live.
    let len = usize::try_from(unsafe { list.as_ref() }.size).unwrap_or(0);
    LIVE.fetch_add(len, Ordering::SeqCst);
    let guard = SdkEventList(list, len);
    // SAFETY: the list is live while the guard lives, and the items are copied before it is dropped.
    let items = unsafe { super::Events::new(guard.0.as_ptr()) }.typed::<E>();
    let mut events = Vec::with_capacity(len);
    let mut skipped = 0;
    for item in items {
        match super::owned::<E>(item) {
            Some(event) => events.push(event),
            None => skipped += 1,
        }
    }
    (events, skipped)
}

/// A list of events with tags that the SDK returned, released on `Drop`.
struct SdkTaggedEventList(NonNull<sys::dxfg_v4_tagged_event_list>, usize);

impl Drop for SdkTaggedEventList {
    fn drop(&mut self) {
        LIVE.fetch_sub(self.1, Ordering::SeqCst);
        if isolate::exiting() {
            return;
        }
        let list = self.0.as_ptr();
        // SAFETY: the list came from the SDK and is released once, with its events and tags; a failed release leaks
        // it.
        crate::error_manager::report_release(
            "EventQueue",
            call(|t| unsafe { sys::dxfg_v4_tagged_event_list_release(t, list) }),
        );
    }
}

/// Copies the events of a list with tags that the SDK gave to the caller, each with its tag, and releases the list;
/// NULL is no events.
///
/// # Safety
///
/// `list` must be NULL or a live SDK list with tags that the caller owns and nothing else releases.
pub(crate) unsafe fn take_tagged_event_list(list: *mut sys::dxfg_v4_tagged_event_list) -> Result<Vec<(i64, Event)>> {
    let Some(list) = NonNull::new(list) else {
        return Ok(Vec::new());
    };
    // SAFETY: the caller guarantees that the list is live.
    let raw = unsafe { list.as_ref() };
    let len = usize::try_from(raw.size).unwrap_or(0);
    LIVE.fetch_add(len, Ordering::SeqCst);
    let guard = SdkTaggedEventList(list, len);
    if len == 0 || raw.elements.is_null() || raw.tags.is_null() {
        return Ok(Vec::new());
    }
    // SAFETY: a live list holds `size` events and `size` tags, live while the guard lives.
    let (elements, tags) = unsafe {
        (
            std::slice::from_raw_parts(raw.elements, len),
            std::slice::from_raw_parts(raw.tags, len),
        )
    };
    let events = elements
        .iter()
        .zip(tags)
        .map(|(&event, &tag)| {
            // SAFETY: the events of a live list are live, and the view is copied before the guard is dropped.
            let event = unsafe { <Event as Sealed>::item(event) }.and_then(|e| e.to_owned());
            event.map(|e| (tag, e)).ok_or_else(unexpected_event)
        })
        .collect();
    drop(guard);
    events
}

/// Copies the events of a list with tags as [`take_tagged_event_list`] does, but skips the events that the crate does
/// not map, as an event stream does, and returns their number with the copies.
///
/// # Safety
///
/// `list` must be NULL or a live SDK list with tags that the caller owns and nothing else releases.
pub(crate) unsafe fn take_owned_tagged_list(list: *mut sys::dxfg_v4_tagged_event_list) -> (Vec<(i64, Event)>, u64) {
    let Some(list) = NonNull::new(list) else {
        return (Vec::new(), 0);
    };
    // SAFETY: the caller guarantees that the list is live.
    let raw = unsafe { list.as_ref() };
    let len = usize::try_from(raw.size).unwrap_or(0);
    LIVE.fetch_add(len, Ordering::SeqCst);
    let guard = SdkTaggedEventList(list, len);
    if len == 0 || raw.elements.is_null() || raw.tags.is_null() {
        return (Vec::new(), 0);
    }
    // SAFETY: a live list holds `size` events and `size` tags, live while the guard lives.
    let (elements, tags) = unsafe {
        (
            std::slice::from_raw_parts(raw.elements, len),
            std::slice::from_raw_parts(raw.tags, len),
        )
    };
    let mut events = Vec::with_capacity(len);
    let mut skipped = 0;
    for (&event, &tag) in elements.iter().zip(tags) {
        // SAFETY: the events of a live list are live, and the view is copied before the guard is dropped.
        match unsafe { <Event as Sealed>::item(event) }.and_then(|e| e.to_owned()) {
            Some(event) => events.push((tag, event)),
            None => skipped += 1,
        }
    }
    drop(guard);
    (events, skipped)
}
