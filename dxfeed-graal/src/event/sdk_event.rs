//! Events the SDK returns outside listener calls, and the single symbols of the last-event requests.

use std::ffi::CString;
use std::ptr::{self, NonNull};
use std::sync::atomic::{AtomicUsize, Ordering};

use dxfeed_graal_sys as sys;

use super::{Event, EventKind, EventType, LastingEvent, sealed::Sealed};
use crate::error::{Error, JavaException, Result, c_string};
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
    fn to_owned<E: LastingEvent>(&self) -> Result<E> {
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
        let _ = call(|t| unsafe { sys::dxfg_v4_EventType_release(t, event) });
    }
}

/// Calls an SDK function that writes a new event or NULL to its out-pointer, and copies the event as `E`.
pub(crate) fn take_event<E: LastingEvent>(
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
        let _ = call(|t| unsafe { sys::dxfg_v4_event_type_list_release(t, list) });
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

/// The symbol of a last-event request for events of type `E`: a candle symbol for the candles, a string otherwise.
pub(crate) enum SdkSymbol {
    /// A string symbol and the text its struct points to.
    String {
        symbol: sys::dxfg_string_symbol_t,
        _text: CString,
    },
    /// A candle symbol the SDK created.
    Candle(NonNull<sys::dxfg_symbol_t>),
}

impl SdkSymbol {
    pub(crate) fn new<E: LastingEvent>(symbol: &str) -> Result<Self> {
        let text = c_string(symbol, "the symbol")?;
        if !needs_candle_symbol(E::TYPE) {
            let string = sys::dxfg_string_symbol_t {
                supper: sys::dxfg_symbol_t { type_: sys::STRING },
                symbol: text.as_ptr(),
            };
            return Ok(SdkSymbol::String {
                symbol: string,
                _text: text,
            });
        }
        // The SDK finds the last candle only by a candle symbol: a string symbol fails with ClassCastException.
        let mut raw = ptr::null_mut();
        // SAFETY: the text is a NUL-terminated string and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_Symbol_new(t, text.as_ptr(), sys::CANDLE, &mut raw) })?;
        let symbol = NonNull::new(raw).ok_or_else(|| Error::Java(JavaException::null_result()))?;
        Ok(SdkSymbol::Candle(symbol))
    }

    /// The SDK symbol, valid while `self` is neither moved nor dropped.
    pub(crate) fn as_mut_ptr(&mut self) -> *mut sys::dxfg_symbol_t {
        match self {
            // The string of the struct is the heap buffer of the `CString`, which does not move with `self`.
            SdkSymbol::String { symbol, .. } => (symbol as *mut sys::dxfg_string_symbol_t).cast(),
            SdkSymbol::Candle(symbol) => symbol.as_ptr(),
        }
    }
}

impl Drop for SdkSymbol {
    fn drop(&mut self) {
        if let SdkSymbol::Candle(symbol) = self
            && !isolate::exiting()
        {
            let symbol = symbol.as_ptr();
            // SAFETY: the symbol came from dxfg_v4_Symbol_new and is released once; a failed release leaks it.
            let _ = call(|t| unsafe { sys::dxfg_v4_Symbol_release(t, symbol) });
        }
    }
}
