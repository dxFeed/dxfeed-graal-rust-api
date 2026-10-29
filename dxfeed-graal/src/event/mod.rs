//! Market events: borrowed views passed to listeners and owned values for publishing.

mod format;
mod quote;
mod trade;

use std::ffi::{CString, c_char};
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

/// The text of Java's `toString` of the event; an event of an unmapped type prints as `Event{class=<code>}`.
impl std::fmt::Display for EventRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EventRef::Quote(q) => q.fmt(f),
            EventRef::Trade(t) => t.fmt(f),
            EventRef::Other(class) => write!(f, "Event{{class={class}}}"),
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

impl Event {
    fn symbol(&self) -> &str {
        match self {
            Event::Quote(q) => &q.symbol,
            Event::Trade(t) => &t.symbol,
        }
    }
}

/// The text of Java's `toString` of the event.
impl std::fmt::Display for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Event::Quote(q) => q.fmt(f),
            Event::Trade(t) => t.fmt(f),
        }
    }
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

/// What a [`DXFeedSubscription`](crate::DXFeedSubscription) delivers: [`Quote`] or [`Trade`] for a subscription to
/// one event type, [`Event`] for a subscription to several.
///
/// The trait is sealed: only this crate implements it.
pub trait SubscriptionItem: sealed::Sealed + Send + 'static {
    /// The item a listener iterates: `&QuoteRef`, `&TradeRef` or [`EventRef`].
    type Item<'a>;
}

/// The event types a subscription can be created for by type: [`Quote`] and [`Trade`]; see
/// [`DXFeed::create_subscription`](crate::DXFeed::create_subscription).
///
/// The trait is sealed: only this crate implements it.
pub trait EventKind: SubscriptionItem {
    /// The event type.
    const TYPE: EventType;
}

mod sealed {
    use super::{SubscriptionItem, sys};

    pub trait Sealed {
        /// Whether every SDK event converts to an item, so that [`Events`](super::Events) knows its exact length.
        const ALL: bool;

        /// Converts an SDK event, or returns `None` for an event of another type.
        ///
        /// # Safety
        ///
        /// `event` must be a live SDK event that outlives `'a`.
        unsafe fn item<'a>(event: *mut sys::dxfg_event_type_t) -> Option<Self::Item<'a>>
        where
            Self: SubscriptionItem;
    }
}

impl sealed::Sealed for Quote {
    const ALL: bool = false;

    unsafe fn item<'a>(event: *mut sys::dxfg_event_type_t) -> Option<&'a QuoteRef> {
        // SAFETY: the caller guarantees the event is live for `'a`; a quote is a `dxfg_quote_t`.
        unsafe { ((*event).clazz == sys::DXFG_EVENT_QUOTE).then(|| &*(event as *const QuoteRef)) }
    }
}

impl SubscriptionItem for Quote {
    type Item<'a> = &'a QuoteRef;
}

impl EventKind for Quote {
    const TYPE: EventType = EventType::Quote;
}

impl sealed::Sealed for Trade {
    const ALL: bool = false;

    unsafe fn item<'a>(event: *mut sys::dxfg_event_type_t) -> Option<&'a TradeRef> {
        // SAFETY: the caller guarantees the event is live for `'a`; a trade is a `dxfg_trade_t`.
        unsafe { ((*event).clazz == sys::DXFG_EVENT_TRADE).then(|| &*(event as *const TradeRef)) }
    }
}

impl SubscriptionItem for Trade {
    type Item<'a> = &'a TradeRef;
}

impl EventKind for Trade {
    const TYPE: EventType = EventType::Trade;
}

impl sealed::Sealed for Event {
    const ALL: bool = true;

    unsafe fn item<'a>(event: *mut sys::dxfg_event_type_t) -> Option<EventRef<'a>> {
        // SAFETY: the caller guarantees the event is live for `'a`; its first field is the class.
        Some(unsafe {
            match (*event).clazz {
                sys::DXFG_EVENT_QUOTE => EventRef::Quote(&*(event as *const QuoteRef)),
                sys::DXFG_EVENT_TRADE => EventRef::Trade(&*(event as *const TradeRef)),
                other => EventRef::Other(other),
            }
        })
    }
}

impl SubscriptionItem for Event {
    type Item<'a> = EventRef<'a>;
}

/// The events of one listener call, borrowed from the SDK: `&QuoteRef` for a subscription to quotes, `&TradeRef` for
/// one to trades and [`EventRef`] for an untyped one.
///
/// A typed iterator skips events of other types; the SDK does not deliver them.
pub struct Events<'a, E: SubscriptionItem = Event> {
    elements: &'a [*mut sys::dxfg_event_type_t],
    next: usize,
    _list: PhantomData<&'a sys::dxfg_event_type_list>,
    _item: PhantomData<fn() -> E>,
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
            _item: PhantomData,
        }
    }

    /// The same events as items of `E`.
    pub(crate) fn typed<E: SubscriptionItem>(self) -> Events<'a, E> {
        Events {
            elements: self.elements,
            next: self.next,
            _list: PhantomData,
            _item: PhantomData,
        }
    }
}

impl<'a, E: SubscriptionItem> Iterator for Events<'a, E> {
    type Item = E::Item<'a>;

    fn next(&mut self) -> Option<E::Item<'a>> {
        while let Some(&event) = self.elements.get(self.next) {
            self.next += 1;
            // SAFETY: the elements of a live list are live events for `'a`.
            if let Some(item) = unsafe { E::item(event) } {
                return Some(item);
            }
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.elements.len() - self.next;
        (if E::ALL { n } else { 0 }, Some(n))
    }
}

impl ExactSizeIterator for Events<'_, Event> {}

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
            let symbol = c_string(event.symbol(), "event symbol")?;
            let sys_event = match event {
                Event::Quote(q) => SysEvent::Quote(q.to_sys(symbol.as_ptr())?),
                Event::Trade(t) => SysEvent::Trade(t.to_sys(symbol.as_ptr())?),
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
