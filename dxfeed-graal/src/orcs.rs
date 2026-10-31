//! The Order Reconstruction Service (`com.dxfeed.orcs`): the price levels and the quotes of a symbol for a period.
//!
//! A [`PriceLevelService`] is a client of the service at an RMI address. It returns the orders that rebuild the price
//! levels of a candle symbol for a period with a granularity, the quotes of the period, and the order sources and
//! symbols the client may request ([`AuthOrderSource`]). [`PriceLevelChecker`] checks the orders it returned.
//!
//! ```no_run
//! use dxfeed_graal::TimeFormat;
//! use dxfeed_graal::event::{CandleSymbol, OrderSource};
//! use dxfeed_graal::orcs::PriceLevelService;
//!
//! let service = PriceLevelService::new("orcs.example.com:7777[login=entitle:<token>]")?;
//! let symbol = CandleSymbol::value_of("AAPL{=d,gr=s}")?;
//! let from = TimeFormat::DEFAULT.parse("20260110-100000")?;
//! let to = TimeFormat::DEFAULT.parse("20260110-230000")?;
//! for order in service.get_orders(&symbol, OrderSource::NTV, from, to, "sample")? {
//!     println!("{order}");
//! }
//! service.close()?;
//! # Ok::<(), dxfeed_graal::Error>(())
//! ```

use std::collections::HashMap;
use std::fmt;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use dxfeed_graal_sys as sys;

use crate::error::{Result, c_string};
use crate::event::sdk_event::take_events;
use crate::event::{CandleSymbol, Event, IndexedEventSource, Order, Quote, SysEvents};
use crate::handle::Handle;
use crate::isolate::{self, call};
use crate::symbol::{Symbol, SymbolArena};

/// A client of the price level service of the Order Reconstruction Service (`PriceLevelService`).
///
/// The client is an RMI client endpoint connected to the address of the service. A request blocks until the service
/// answers or fails; while the service is not connected, the request fails after the
/// `com.devexperts.rmi.RequestSendingTimeout` system property, in milliseconds, a minute by default. [`close`](Self::close)
/// or dropping the client closes the connection.
pub struct PriceLevelService {
    handle: Handle<sys::dxfg_v4_price_level_service_t>,
    closed: AtomicBool,
}

impl PriceLevelService {
    /// Creates a client of the service at an RMI `address`, such as `orcs.example.com:7777`; an authorization token
    /// goes to the address as `<address>[login=entitle:<token>]` (C++ `PriceLevelService::create`; in Java, the proxy
    /// of `PriceLevelService` of an RMI client endpoint connected to the address).
    #[doc(alias = "create")]
    pub fn new(address: &str) -> Result<PriceLevelService> {
        let address = c_string(address, "the address")?;
        // SAFETY: the address is live and the out-pointer is valid for writes.
        let handle = Handle::create(|t, out| unsafe { sys::dxfg_v4_PriceLevelService_new(t, address.as_ptr(), out) })?;
        Ok(PriceLevelService {
            handle,
            closed: AtomicBool::new(false),
        })
    }

    /// The orders that rebuild the price levels of `candle_symbol` from `order_source` within the time range, sorted
    /// by time (`getOrders`). Times are in milliseconds since the Unix epoch; `caller` names the client to the service.
    ///
    /// The candle symbol has a period and the granularity attribute `gr` in seconds, at least 1, such as
    /// `AAPL{=d,gr=s}`. The list is empty when the service does not support the period or the granularity. The source
    /// is an order source, not [`IndexedEventSource::DEFAULT`].
    #[doc(alias = "getOrders")]
    pub fn get_orders(
        &self,
        candle_symbol: &CandleSymbol,
        order_source: IndexedEventSource,
        from_time: i64,
        to_time: i64,
        caller: &str,
    ) -> Result<Vec<Order>> {
        let mut arena = SymbolArena::new();
        let symbol = arena.symbol(&Symbol::Candle(candle_symbol.clone()), true)?;
        let source = arena.source(order_source);
        let caller = c_string(caller, "the caller")?;
        let s = self.handle.as_ptr();
        // SAFETY: the service, the symbol, the source and the caller are live; the out-pointer is valid for writes.
        take_events(|t, out| unsafe {
            sys::dxfg_v4_PriceLevelService_getOrdersWithCaller(
                t,
                s,
                symbol,
                source,
                from_time,
                to_time,
                caller.as_ptr(),
                out,
            )
        })
        .map(Option::unwrap_or_default)
    }

    /// The quotes of `candle_symbol` within the time range, sorted by time (`getQuotes`). Times are in milliseconds
    /// since the Unix epoch; `caller` names the client to the service.
    #[doc(alias = "getQuotes")]
    pub fn get_quotes(
        &self,
        candle_symbol: &CandleSymbol,
        from_time: i64,
        to_time: i64,
        caller: &str,
    ) -> Result<Vec<Quote>> {
        let mut arena = SymbolArena::new();
        let symbol = arena.symbol(&Symbol::Candle(candle_symbol.clone()), true)?;
        let caller = c_string(caller, "the caller")?;
        let s = self.handle.as_ptr();
        // SAFETY: the service, the symbol and the caller are live; the out-pointer is valid for writes.
        take_events(|t, out| unsafe {
            sys::dxfg_v4_PriceLevelService_getQuotesWithCaller(t, s, symbol, from_time, to_time, caller.as_ptr(), out)
        })
        .map(Option::unwrap_or_default)
    }

    /// The order sources and their symbols that the client may request, as of now (`getAuthOrderSource`).
    #[doc(alias = "getAuthOrderSource")]
    pub fn get_auth_order_source(&self) -> Result<AuthOrderSource> {
        let s = self.handle.as_ptr();
        // SAFETY: the service is live and the out-pointer is valid for writes.
        let handle = Handle::create(|t, out| unsafe { sys::dxfg_v4_PriceLevelService_getAuthOrderSource(t, s, out) })?;
        Ok(AuthOrderSource { handle })
    }

    /// Closes the connection to the service; later requests fail (`close`). Closing again does nothing.
    pub fn close(&self) -> Result<()> {
        if self.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let s = self.handle.as_ptr();
        // SAFETY: the service is live.
        call(|t| unsafe { sys::dxfg_v4_PriceLevelService_close(t, s) })
    }

    /// Whether [`close`](Self::close) was called.
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }
}

impl Drop for PriceLevelService {
    fn drop(&mut self) {
        if isolate::exiting() {
            // No SDK calls from destructors while the process exits.
            return;
        }
        let _ = self.close();
    }
}

impl fmt::Debug for PriceLevelService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PriceLevelService")
            .field("closed", &self.is_closed())
            .finish_non_exhaustive()
    }
}

/// The order sources and their symbols that a client of the [`PriceLevelService`] may request
/// (`AuthOrderSource`).
pub struct AuthOrderSource {
    handle: Handle<sys::dxfg_v4_auth_order_source_t>,
}

impl AuthOrderSource {
    /// The symbols of each available order source, by the id of the source (`getByIds`).
    #[doc(alias = "getByIds")]
    pub fn get_by_ids(&self) -> Result<HashMap<i32, Vec<String>>> {
        let mut list = ptr::null_mut();
        let a = self.handle.as_ptr();
        // SAFETY: the object is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_AuthOrderSource_getByIds(t, a, &mut list) })?;
        // SAFETY: the SDK filled the list, which is copied before it is released below.
        let entries = unsafe { elements(list.as_ref().map(|l| (l.elements, l.size))) }
            .iter()
            // SAFETY: the entries of the list are live.
            .map(|&entry| unsafe { ((*entry).order_source_id, strings((*entry).symbols)) })
            .collect();
        if !list.is_null() {
            // SAFETY: the list is released once, after the copy.
            let _ = call(|t| unsafe { sys::dxfg_v4_symbols_by_order_source_id_map_entry_list_release(t, list) });
        }
        Ok(entries)
    }

    /// The symbols of each available order source (`getByOrderSources`).
    #[doc(alias = "getByOrderSources")]
    pub fn get_by_order_sources(&self) -> Result<HashMap<IndexedEventSource, Vec<String>>> {
        let mut list = ptr::null_mut();
        let a = self.handle.as_ptr();
        // SAFETY: the object is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_AuthOrderSource_getByOrderSources(t, a, &mut list) })?;
        // SAFETY: the SDK filled the list, which is copied before it is released below.
        let entries = unsafe { elements(list.as_ref().map(|l| (l.elements, l.size))) }
            .iter()
            .map(|&entry| {
                // SAFETY: the entries of the list and their sources are live.
                let (source, symbols) = unsafe { ((*entry).order_source, (*entry).symbols) };
                // SAFETY: as above.
                let id = unsafe { source.as_ref() }.map_or(0, |source| source.id);
                // SAFETY: as above.
                IndexedEventSource::from_id(id).map(|source| (source, unsafe { strings(symbols) }))
            })
            .collect();
        if !list.is_null() {
            // SAFETY: the list is released once, after the copy.
            let _ = call(|t| unsafe { sys::dxfg_v4_symbols_by_order_source_map_entry_list_release(t, list) });
        }
        entries
    }
}

impl fmt::Debug for AuthOrderSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthOrderSource").finish_non_exhaustive()
    }
}

/// Checks the orders of the [`PriceLevelService`] (`PriceLevelChecker`).
#[derive(Debug)]
pub struct PriceLevelChecker;

impl PriceLevelChecker {
    /// Whether the bid prices of `orders` stay below their ask prices at every moment and no gap between the times of
    /// two subsequent orders exceeds `time_gap_bound` (`validate`). Pass the orders sorted by time, as
    /// [`PriceLevelService::get_orders`] returns them; an empty list is valid. The details of the check, and the quotes
    /// with `print_quotes`, go to the log, not to the caller. Note that sometimes a bid at or above the ask is normal.
    pub fn validate(orders: &[Order], time_gap_bound: Duration, print_quotes: bool) -> Result<bool> {
        let events: Vec<Event> = orders.iter().cloned().map(Event::from).collect();
        let mut events = SysEvents::new(&events)?;
        let mut list = events.list();
        let bound = i64::try_from(time_gap_bound.as_millis()).unwrap_or(i64::MAX);
        let mut valid = 0;
        // SAFETY: the list and its events are live; the SDK only reads them; the out-pointer is valid for writes.
        call(|t| unsafe {
            sys::dxfg_v4_PriceLevelChecker_validate(t, &mut list, bound, i32::from(print_quotes), &mut valid)
        })?;
        Ok(valid != 0)
    }
}

/// The elements of an SDK list, or none for NULL or a non-positive size.
///
/// # Safety
///
/// The elements must be live while the slice is used.
unsafe fn elements<'a, T>(list: Option<(*mut T, i32)>) -> &'a [T] {
    match list {
        Some((elements, size)) if size > 0 && !elements.is_null() => {
            // SAFETY: the caller guarantees the elements are live.
            unsafe { std::slice::from_raw_parts(elements, size as usize) }
        }
        _ => &[],
    }
}

/// A copy of an SDK string list, empty for NULL.
///
/// # Safety
///
/// The list, if not NULL, must be live.
unsafe fn strings(list: *mut sys::dxfg_string_list) -> Vec<String> {
    // SAFETY: the caller guarantees the list is live.
    unsafe { elements(list.as_ref().map(|l| (l.elements, l.size))) }
        .iter()
        // SAFETY: the strings of a live list are live.
        .map(|&s| unsafe { isolate::string(s) }.into_owned())
        .collect()
}
