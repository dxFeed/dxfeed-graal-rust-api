//! Market events: borrowed views passed to listeners and owned values for publishing (KTD9).

mod quote;
mod trade;

use std::borrow::Cow;
use std::ffi::{CStr, CString, c_char};
use std::marker::PhantomData;

use dxfeed_graal_sys as sys;

pub use quote::{Quote, QuoteRef};
pub use trade::{Trade, TradeRef};

use crate::error::{Error, Result, c_string};

/// An event type a subscription can receive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum EventType {
    /// [`Quote`]: the best bid and offer.
    Quote,
    /// [`Trade`]: the last trade.
    Trade,
}

impl EventType {
    pub(crate) fn to_sys(self) -> sys::dxfg_event_clazz_t {
        match self {
            EventType::Quote => sys::DXFG_EVENT_QUOTE,
            EventType::Trade => sys::DXFG_EVENT_TRADE,
        }
    }
}

/// An event received by a listener, valid during the callback only.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum EventRef<'a> {
    /// A quote.
    Quote(&'a QuoteRef),
    /// A trade.
    Trade(&'a TradeRef),
    /// An event of a type this crate does not map yet, with its SDK class code (`dxfg_event_clazz_t`).
    Other(i32),
}

impl EventRef<'_> {
    /// Copies the event, or returns `None` for [`EventRef::Other`].
    pub fn to_owned(&self) -> Option<Event> {
        match *self {
            EventRef::Quote(q) => Some(Event::Quote(q.to_owned())),
            EventRef::Trade(t) => Some(Event::Trade(t.to_owned())),
            EventRef::Other(_) => None,
        }
    }
}

/// An owned event, e.g. for publishing.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Event {
    /// A quote.
    Quote(Quote),
    /// A trade.
    Trade(Trade),
}

impl From<Quote> for Event {
    fn from(quote: Quote) -> Self {
        Event::Quote(quote)
    }
}

impl From<Trade> for Event {
    fn from(trade: Trade) -> Self {
        Event::Trade(trade)
    }
}

/// The events of one listener call: an iterator of [`EventRef`] borrowed from the SDK.
pub struct Events<'a> {
    elements: &'a [*mut sys::dxfg_event_type_t],
    next: usize,
    _list: PhantomData<&'a sys::dxfg_event_type_list>,
}

impl<'a> Events<'a> {
    /// # Safety
    ///
    /// `list` must be NULL or a live SDK event list that outlives `'a`.
    pub(crate) unsafe fn new(list: *const sys::dxfg_event_type_list) -> Self {
        // SAFETY: the caller guarantees `list` is NULL or live for `'a`.
        let elements = match unsafe { list.as_ref() } {
            // SAFETY: a live list holds `size` element pointers.
            Some(list) if list.size > 0 && !list.elements.is_null() => unsafe {
                std::slice::from_raw_parts(list.elements, list.size as usize)
            },
            _ => &[],
        };
        Events {
            elements,
            next: 0,
            _list: PhantomData,
        }
    }
}

impl<'a> Iterator for Events<'a> {
    type Item = EventRef<'a>;

    fn next(&mut self) -> Option<EventRef<'a>> {
        let event = *self.elements.get(self.next)?;
        self.next += 1;
        // SAFETY: the elements of a live list are live events whose first field is the class.
        Some(unsafe {
            match (*event).clazz {
                sys::DXFG_EVENT_QUOTE => EventRef::Quote(&*(event as *const QuoteRef)),
                sys::DXFG_EVENT_TRADE => EventRef::Trade(&*(event as *const TradeRef)),
                other => EventRef::Other(other),
            }
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.elements.len() - self.next;
        (n, Some(n))
    }
}

impl ExactSizeIterator for Events<'_> {}

/// The symbol of an SDK event; empty if NULL, lossy if not UTF-8.
///
/// # Safety
///
/// `symbol` must be NULL or a NUL-terminated string that outlives `'a`.
unsafe fn symbol<'a>(symbol: *const c_char) -> Cow<'a, str> {
    if symbol.is_null() {
        Cow::Borrowed("")
    } else {
        // SAFETY: the caller guarantees a NUL-terminated string.
        unsafe { CStr::from_ptr(symbol) }.to_string_lossy()
    }
}

/// A Java `char` exchange code from the SDK.
fn exchange_code(code: i16) -> char {
    char::from_u32(code as u16 as u32).unwrap_or(char::REPLACEMENT_CHARACTER)
}

/// A Java `char` exchange code for the SDK; it must fit 16 bits.
fn sys_exchange_code(code: char) -> Result<i16> {
    u16::try_from(code as u32)
        .map(|c| c as i16)
        .map_err(|_| Error::InvalidArgument(format!("exchange code {code:?} does not fit a Java char")))
}

/// SDK events built from owned ones, with the strings they point to; kept alive while the SDK reads them.
pub(crate) struct SysEvents {
    events: Vec<SysEvent>,
    _symbols: Vec<CString>,
    pointers: Vec<*mut sys::dxfg_event_type_t>,
}

enum SysEvent {
    Quote(sys::dxfg_quote_t),
    Trade(sys::dxfg_trade_t),
}

impl SysEvents {
    pub(crate) fn new(events: &[Event]) -> Result<Self> {
        let mut symbols = Vec::with_capacity(events.len());
        let mut sys_events = Vec::with_capacity(events.len());
        for event in events {
            let (symbol, sys_event) = match event {
                Event::Quote(q) => {
                    let symbol = c_string(&q.symbol, "event symbol")?;
                    let raw = q.to_sys(symbol.as_ptr())?;
                    (symbol, SysEvent::Quote(raw))
                }
                Event::Trade(t) => {
                    let symbol = c_string(&t.symbol, "event symbol")?;
                    let raw = t.to_sys(symbol.as_ptr())?;
                    (symbol, SysEvent::Trade(raw))
                }
            };
            symbols.push(symbol);
            sys_events.push(sys_event);
        }
        let pointers = sys_events
            .iter_mut()
            .map(|e| match e {
                SysEvent::Quote(q) => (q as *mut sys::dxfg_quote_t).cast(),
                SysEvent::Trade(t) => (t as *mut sys::dxfg_trade_t).cast(),
            })
            .collect();
        Ok(SysEvents {
            events: sys_events,
            _symbols: symbols,
            pointers,
        })
    }

    /// The SDK list over the events, valid while `self` is neither moved nor dropped.
    pub(crate) fn list(&mut self) -> sys::dxfg_event_type_list {
        debug_assert_eq!(self.events.len(), self.pointers.len());
        sys::dxfg_event_type_list {
            size: self.pointers.len() as i32,
            elements: self.pointers.as_mut_ptr(),
        }
    }
}

fn market_event(clazz: sys::dxfg_event_clazz_t, symbol: *const c_char, event_time: i64) -> sys::dxfg_market_event_t {
    sys::dxfg_market_event_t {
        event_type: sys::dxfg_event_type_t { clazz },
        event_symbol: symbol,
        event_time,
    }
}
