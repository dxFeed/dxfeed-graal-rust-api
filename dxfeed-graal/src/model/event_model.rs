//! The live lists of indexed and time-series events of one symbol (`IndexedEventModel`, `TimeSeriesEventModel`).

use std::fmt;
use std::marker::PhantomData;
use std::ptr;
use std::sync::Mutex;

use dxfeed_graal_sys as sys;

use crate::error::Result;
use crate::error_manager::report_release;
use crate::event::sdk_event::needs_candle_symbol;
use crate::event::{EventKind, IndexedEvent, TimeSeriesEvent};
use crate::executor::Executor;
use crate::feed::DXFeed;
use crate::handle::Handle;
use crate::isolate::{self, call};
use crate::subscription_controller::{attach_feed, detach_feed};
use crate::symbol::{Symbol, SymbolArena};

use super::ObservableListModel;

/// The methods both models share, over the SDK functions of the model.
macro_rules! event_model {
    (
        $model:ident<$bound:path>, $java:literal, $what:literal,
        new: $new:ident, attach: $attach:ident, detach: $detach:ident, close: $close:ident,
        get_executor: $get_executor:ident, set_executor: $set_executor:ident, clear: $clear:ident,
        clear_doc: $clear_doc:literal,
        get_symbol: $get_symbol:ident, set_symbol: $set_symbol:ident,
        get_size_limit: $get_size_limit:ident, set_size_limit: $set_size_limit:ident,
        get_events_list: $get_events_list:ident
    ) => {
        impl<E: $bound + EventKind> $model<E> {
            #[doc = concat!(
                        "A new model of the ", $what, " of `E` without a symbol and without a size limit (`", $java,
                        "(Class)`)."
                    )]
            #[doc = ""]
            #[doc = "Set its symbol and attach it to a feed to fill it."]
            #[doc(alias = $java)]
            pub fn new() -> Result<Self> {
                // SAFETY: the out-pointer is valid for writes.
                let handle = Handle::create(|t, out| unsafe { sys::$new(t, E::TYPE.to_sys(), out) })?;
                Ok($model {
                    handle,
                    feeds: Mutex::new(Vec::new()),
                    _event: PhantomData,
                })
            }

            /// Attaches the model to `feed`, which then delivers the events of its symbol (`attach`). The model keeps
            /// the feed until it is detached or closed.
            pub fn attach(&self, feed: &DXFeed) -> Result<()> {
                let m = self.handle.as_ptr();
                attach_feed(&self.feeds, feed, |f| {
                    // SAFETY: the model and the feed are live.
                    call(|t| unsafe { sys::$attach(t, m, f) })?;
                    Ok(true)
                })
            }

            /// Detaches the model from `feed` and lets go of it (`detach`).
            pub fn detach(&self, feed: &DXFeed) -> Result<()> {
                let m = self.handle.as_ptr();
                detach_feed(&self.feeds, feed, |f| {
                    // SAFETY: the model and the feed are live.
                    call(|t| unsafe { sys::$detach(t, m, f) })
                })
            }

            /// Detaches the model from all feeds for good, lets go of them and removes the listeners of its list;
            /// closing it again does nothing (`close`). Dropping the model closes it too.
            pub fn close(&self) -> Result<()> {
                let mut feeds = crate::lock(&self.feeds);
                self.close_model()?;
                feeds.clear();
                Ok(())
            }

            fn close_model(&self) -> Result<()> {
                let m = self.handle.as_ptr();
                // SAFETY: the model is live.
                call(|t| unsafe { sys::$close(t, m) })
            }

            /// The executor that calls the listeners of the list; `None` if it is the executor of the feed
            /// (`getExecutor`).
            #[doc(alias = "getExecutor")]
            pub fn get_executor(&self) -> Result<Option<Executor>> {
                let m = self.handle.as_ptr();
                // SAFETY: the model is live and the out-pointer is valid for writes.
                let handle = Handle::create_optional(|t, out| unsafe { sys::$get_executor(t, m, out) })?;
                Ok(handle.map(Executor::from_handle))
            }

            /// Calls the listeners of the list on `executor` from now on, or on the executor of the feed for `None`
            /// (`setExecutor`). The model keeps the executor.
            #[doc(alias = "setExecutor")]
            pub fn set_executor(&self, executor: Option<&Executor>) -> Result<()> {
                let m = self.handle.as_ptr();
                let e = executor.map_or(ptr::null_mut(), |executor| executor.handle.as_ptr());
                // SAFETY: the model and the executor are live.
                call(|t| unsafe { sys::$set_executor(t, m, e) })
            }

            #[doc = $clear_doc]
            pub fn clear(&self) -> Result<()> {
                let m = self.handle.as_ptr();
                // SAFETY: the model is live.
                call(|t| unsafe { sys::$clear(t, m) })
            }

            /// The symbol set with [`set_symbol`](Self::set_symbol), or `None` for none (`getSymbol`). A model of
            /// candles returns a symbol set from a string as a [`Symbol::Candle`].
            #[doc(alias = "getSymbol")]
            pub fn get_symbol(&self) -> Result<Option<Symbol>> {
                let m = self.handle.as_ptr();
                let mut symbol = ptr::null_mut();
                // SAFETY: the model is live and the out-pointer is valid for writes.
                call(|t| unsafe { sys::$get_symbol(t, m, &mut symbol) })?;
                if symbol.is_null() {
                    return Ok(None);
                }
                // SAFETY: the SDK wrote a live symbol struct, which is ours to release.
                let result = unsafe { Symbol::from_sys(symbol) };
                if result.is_none() {
                    crate::ErrorManager::report($java, "the symbol is of a kind or source this crate does not know");
                }
                // SAFETY: the symbol is live and released once.
                let released = call(|t| unsafe { sys::dxfg_v4_Symbol_release(t, symbol) });
                report_release($java, released);
                Ok(result)
            }

            /// Subscribes the model to `symbol` instead of its symbol, or unsubscribes it for `None`, which removes
            /// its events (`setSymbol`).
            ///
            /// Pass `None::<Symbol>` to unsubscribe. Returns [`Error::InvalidArgument`](crate::Error::InvalidArgument)
            /// if the symbol has a NUL byte.
            #[doc(alias = "setSymbol")]
            pub fn set_symbol(&self, symbol: Option<impl Into<Symbol>>) -> Result<()> {
                let m = self.handle.as_ptr();
                let mut arena = SymbolArena::new();
                let symbol = match symbol {
                    Some(symbol) => arena.symbol(&symbol.into(), needs_candle_symbol(E::TYPE))?,
                    None => ptr::null_mut(),
                };
                // SAFETY: the model is live and the symbol lives in the arena during the call.
                call(|t| unsafe { sys::$set_symbol(t, m, symbol) })
            }

            /// The most events the list keeps, `i32::MAX` (the default) for no limit (`getSizeLimit`).
            #[doc(alias = "getSizeLimit")]
            pub fn get_size_limit(&self) -> Result<i32> {
                let m = self.handle.as_ptr();
                let mut limit = 0;
                // SAFETY: the model is live and the out-pointer is valid for writes.
                call(|t| unsafe { sys::$get_size_limit(t, m, &mut limit) })?;
                Ok(limit)
            }

            /// Keeps at most `limit` events in the list, dropping the first ones (`setSizeLimit`); a negative limit
            /// is an [`Error::Java`](crate::Error::Java). As in Java, the list shrinks at once, but its listeners
            /// are not called until its next change.
            #[doc(alias = "setSizeLimit")]
            pub fn set_size_limit(&self, limit: i32) -> Result<()> {
                let m = self.handle.as_ptr();
                // SAFETY: the model is live.
                call(|t| unsafe { sys::$set_size_limit(t, m, limit) })
            }

            /// The live list of the events of the model (`getEventsList`). Each call returns a new handle of the same
            /// list.
            #[doc(alias = "getEventsList")]
            pub fn get_events_list(&self) -> Result<ObservableListModel<E>> {
                let m = self.handle.as_ptr();
                // SAFETY: the model is live and the out-pointer is valid for writes.
                let handle = Handle::create(|t, out| unsafe { sys::$get_events_list(t, m, out) })?;
                Ok(ObservableListModel::new(handle))
            }
        }

        impl<E: $bound + EventKind> Drop for $model<E> {
            fn drop(&mut self) {
                if isolate::exiting() {
                    // No SDK calls while the process exits.
                    return;
                }
                report_release($java, self.close_model());
            }
        }

        impl<E: $bound + EventKind> fmt::Debug for $model<E> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_struct($java)
                    .field("event_type", &E::TYPE)
                    .finish_non_exhaustive()
            }
        }
    };
}

/// A live list of the indexed events of one symbol, such as the series of an option (`IndexedEventModel`).
///
/// The list, from [`get_events_list`](Self::get_events_list), holds the events of all sources in the order of their
/// source id and index and drops the removed ones; its listeners are called after each change. As in Java, the model
/// keeps only the events whose event symbol equals its symbol, so subscribe it to a plain symbol: with an
/// [`IndexedEventSubscription`](Symbol::IndexedEventSubscription) symbol it receives no events.
///
/// As in Java, the model is not thread-safe apart from [`attach`](Self::attach), [`detach`](Self::detach) and
/// [`close`](Self::close): change it and read its list in the listeners of the list, or on the thread that runs a
/// [`ManualExecutor`](crate::ManualExecutor) of the model. Changing its symbol calls the listeners of the list on
/// the calling thread.
///
/// Dropping the model closes it. It keeps the endpoints of the feeds it is attached to open.
///
/// ```no_run
/// use dxfeed_graal::DXFeed;
/// use dxfeed_graal::event::Series;
/// use dxfeed_graal::model::IndexedEventModel;
///
/// # fn main() -> dxfeed_graal::Result<()> {
/// let model = IndexedEventModel::<Series>::new()?;
/// model.set_symbol(Some("AAPL"))?;
/// let series = model.get_events_list()?;
/// series.add_listener(|series| println!("{} series", series.count()))?;
/// model.attach(DXFeed::get_instance()?)?;
/// # Ok(())
/// # }
/// ```
pub struct IndexedEventModel<E: IndexedEvent + EventKind> {
    handle: Handle<sys::dxfg_v4_indexed_event_model_t>,
    /// The feeds the model is attached to; dropped last, as they may close their endpoints.
    feeds: Mutex<Vec<DXFeed>>,
    _event: PhantomData<fn() -> E>,
}

event_model!(
    IndexedEventModel<IndexedEvent>, "IndexedEventModel", "indexed events",
    new: dxfg_v4_IndexedEventModel_new, attach: dxfg_v4_IndexedEventModel_attach,
    detach: dxfg_v4_IndexedEventModel_detach, close: dxfg_v4_IndexedEventModel_close,
    get_executor: dxfg_v4_IndexedEventModel_getExecutor, set_executor: dxfg_v4_IndexedEventModel_setExecutor,
    clear: dxfg_v4_IndexedEventModel_clear,
    clear_doc: "Removes all the events from the model and unsubscribes it, as `set_symbol(None)` (`clear`).",
    get_symbol: dxfg_v4_IndexedEventModel_getSymbol, set_symbol: dxfg_v4_IndexedEventModel_setSymbol,
    get_size_limit: dxfg_v4_IndexedEventModel_getSizeLimit, set_size_limit: dxfg_v4_IndexedEventModel_setSizeLimit,
    get_events_list: dxfg_v4_IndexedEventModel_getEventsList
);

/// A live list of the time-series events of one symbol from a time on, such as the candles of a chart
/// (`TimeSeriesEventModel`).
///
/// The list, from [`get_events_list`](Self::get_events_list), holds the events in the order of their time; its
/// listeners are called after each change. The model receives events once it has a symbol and a time from
/// [`set_from_time`](Self::set_from_time). As in Java, it keeps only the events whose event symbol equals its
/// symbol, so subscribe it to a plain or candle symbol: with a
/// [`TimeSeriesSubscription`](Symbol::TimeSeriesSubscription) symbol it receives no events.
///
/// As in Java, the model is not thread-safe apart from [`attach`](Self::attach), [`detach`](Self::detach) and
/// [`close`](Self::close): change it and read its list in the listeners of the list, or on the thread that runs a
/// [`ManualExecutor`](crate::ManualExecutor) of the model. Changing its symbol calls the listeners of the list on
/// the calling thread.
///
/// Dropping the model closes it. It keeps the endpoints of the feeds it is attached to open.
///
/// ```no_run
/// use dxfeed_graal::DXFeed;
/// use dxfeed_graal::event::Candle;
/// use dxfeed_graal::model::TimeSeriesEventModel;
///
/// # fn main() -> dxfeed_graal::Result<()> {
/// let model = TimeSeriesEventModel::<Candle>::new()?;
/// model.set_symbol(Some("AAPL{=1d}"))?;
/// model.set_from_time(0)?;
/// let candles = model.get_events_list()?;
/// candles.add_listener(|candles| println!("{} candles", candles.count()))?;
/// model.attach(DXFeed::get_instance()?)?;
/// # Ok(())
/// # }
/// ```
pub struct TimeSeriesEventModel<E: TimeSeriesEvent + EventKind> {
    handle: Handle<sys::dxfg_v4_time_series_event_model_t>,
    /// The feeds the model is attached to; dropped last, as they may close their endpoints.
    feeds: Mutex<Vec<DXFeed>>,
    _event: PhantomData<fn() -> E>,
}

event_model!(
    TimeSeriesEventModel<TimeSeriesEvent>, "TimeSeriesEventModel", "time-series events",
    new: dxfg_v4_TimeSeriesEventModel_new, attach: dxfg_v4_TimeSeriesEventModel_attach,
    detach: dxfg_v4_TimeSeriesEventModel_detach, close: dxfg_v4_TimeSeriesEventModel_close,
    get_executor: dxfg_v4_TimeSeriesEventModel_getExecutor, set_executor: dxfg_v4_TimeSeriesEventModel_setExecutor,
    clear: dxfg_v4_TimeSeriesEventModel_clear,
    clear_doc: "Removes all the events from the model and unsubscribes it, as `set_symbol(None)`, and resets the \
                from-time to `i64::MAX` (`clear`).",
    get_symbol: dxfg_v4_TimeSeriesEventModel_getSymbol, set_symbol: dxfg_v4_TimeSeriesEventModel_setSymbol,
    get_size_limit: dxfg_v4_TimeSeriesEventModel_getSizeLimit,
    set_size_limit: dxfg_v4_TimeSeriesEventModel_setSizeLimit,
    get_events_list: dxfg_v4_TimeSeriesEventModel_getEventsList
);

impl<E: TimeSeriesEvent + EventKind> TimeSeriesEventModel<E> {
    /// The time in milliseconds since the epoch from which the model receives events; `i64::MAX` (the default)
    /// if it receives none (`getFromTime`).
    #[doc(alias = "getFromTime")]
    pub fn get_from_time(&self) -> Result<i64> {
        let m = self.handle.as_ptr();
        let mut from_time = 0;
        // SAFETY: the model is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_TimeSeriesEventModel_getFromTime(t, m, &mut from_time) })?;
        Ok(from_time)
    }

    /// Receives the events from `from_time`, in milliseconds since the epoch, on (`setFromTime`); `i64::MAX`
    /// unsubscribes the model.
    ///
    /// As in Java, a new time of a model that already has a symbol and a time does not renew its subscription:
    /// set `i64::MAX` first to receive the events from the new time.
    #[doc(alias = "setFromTime")]
    pub fn set_from_time(&self, from_time: i64) -> Result<()> {
        let m = self.handle.as_ptr();
        // SAFETY: the model is live.
        call(|t| unsafe { sys::dxfg_v4_TimeSeriesEventModel_setFromTime(t, m, from_time) })
    }
}
