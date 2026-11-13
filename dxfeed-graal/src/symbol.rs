//! The symbols of subscriptions (`WildcardSymbol`, `IndexedEventSubscriptionSymbol`, `TimeSeriesSubscriptionSymbol`).

use std::borrow::Cow;
use std::ffi::{CString, c_char};
use std::fmt::{self, Display, Formatter};

use dxfeed_graal_sys as sys;

use crate::TimeFormat;
use crate::error::{Result, c_string};
use crate::event::{CandleSymbol, IndexedEventSource};
use crate::isolate;

/// A symbol that a subscription adds or removes, or that a feed request takes.
///
/// A string converts into it, so the methods that take symbols take strings too. The kinds besides strings are those
/// of the Java API:
///
/// - [`Candle`](Self::Candle): a [`CandleSymbol`], such as `AAPL{=d}`;
/// - [`Wildcard`](Self::Wildcard): all the symbols, on endpoints with `dxfeed.wildcard.enable=true`
///   (`WildcardSymbol.ALL`);
/// - [`IndexedEventSubscription`](Self::IndexedEventSubscription): the indexed events of a symbol from one source,
///   such as the orders from `NTV` (`IndexedEventSubscriptionSymbol`);
/// - [`TimeSeriesSubscription`](Self::TimeSeriesSubscription): the time-series events of a symbol from a time on
///   (`TimeSeriesSubscriptionSymbol`).
///
/// # Examples
///
/// String slices and `String`s convert into it, so plain symbols need no `Symbol`:
///
/// ```
/// use dxfeed_graal::Symbol;
///
/// let symbols: Vec<Symbol> = ["AAPL", "IBM"].into_iter().map(Symbol::from).collect();
/// assert_eq!(symbols[0], Symbol::String("AAPL".into()));
/// assert_eq!(Symbol::from(String::from("IBM")).to_string(), "IBM");
/// ```
///
/// An indexed symbol subscribes to the indexed events of one source only, such as the orders of `AAPL` from `NTV`
/// (a plain symbol receives the events of all the sources):
///
/// ```no_run
/// # fn main() -> dxfeed_graal::Result<()> {
/// use dxfeed_graal::event::{Order, OrderSource};
/// use dxfeed_graal::{DXFeed, Symbol};
///
/// let subscription = DXFeed::get_instance()?.create_subscription::<Order>()?;
/// subscription.add_event_listener(|orders| orders.for_each(|order| println!("{order}")))?;
/// subscription.add_symbols([Symbol::indexed("AAPL", OrderSource::NTV)])?;
/// # Ok(())
/// # }
/// ```
///
/// The parts of an indexed symbol:
///
/// ```
/// use dxfeed_graal::Symbol;
/// use dxfeed_graal::event::OrderSource;
///
/// let symbol = Symbol::indexed("AAPL", OrderSource::NTV);
/// assert_eq!(symbol.to_string(), "AAPL{source=NTV}");
/// assert_eq!(symbol.get_event_symbol(), &Symbol::from("AAPL"));
/// assert_eq!(symbol.get_source(), Some(OrderSource::NTV));
/// ```
///
/// The wildcard symbol subscribes to all the symbols, on an endpoint that enables it:
///
/// ```no_run
/// # fn main() -> dxfeed_graal::Result<()> {
/// use dxfeed_graal::event::Trade;
/// use dxfeed_graal::{DXEndpoint, Symbol};
///
/// let endpoint = DXEndpoint::new_builder()
///     .with_property(DXEndpoint::DXFEED_WILDCARD_ENABLE_PROPERTY, "true")
///     .build()?;
/// let subscription = endpoint.get_feed().create_subscription::<Trade>()?;
/// subscription.add_event_listener(|trades| trades.for_each(|trade| println!("{trade}")))?;
/// subscription.add_symbols([Symbol::Wildcard])?;
/// // Connect the endpoint to an address that serves wildcard subscriptions.
/// # Ok(())
/// # }
/// ```
///
/// A time-series symbol subscribes to the events from a time on; a
/// [`DXFeedTimeSeriesSubscription`](crate::DXFeedTimeSeriesSubscription) wraps its symbols in it by itself:
///
/// ```no_run
/// # fn main() -> dxfeed_graal::Result<()> {
/// use dxfeed_graal::event::{Candle, CandlePeriod, CandleSymbol};
/// use dxfeed_graal::{DXEndpoint, Symbol, TimeFormat};
///
/// let subscription = DXEndpoint::create()?.get_feed().create_subscription::<Candle>()?;
/// let symbol = CandleSymbol::value_of_with("AAPL", [CandlePeriod::DAY])?;
/// subscription.add_symbols([Symbol::time_series(symbol, TimeFormat::DEFAULT.parse("20260101")?)])?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Symbol {
    /// A string symbol, such as `AAPL`.
    String(String),
    /// A candle symbol, such as `AAPL{=d}`.
    Candle(CandleSymbol),
    /// All the symbols (`WildcardSymbol.ALL`), printed as `*`.
    Wildcard,
    /// The indexed events of `symbol` from `source` (`IndexedEventSubscriptionSymbol`).
    IndexedEventSubscription {
        /// The symbol of the events.
        symbol: Box<Symbol>,
        /// The source of the events.
        source: IndexedEventSource,
    },
    /// The time-series events of `symbol` from `from_time` on, in milliseconds since the Unix epoch
    /// (`TimeSeriesSubscriptionSymbol`).
    TimeSeriesSubscription {
        /// The symbol of the events.
        symbol: Box<Symbol>,
        /// The time of the first event.
        from_time: i64,
    },
}

impl Symbol {
    /// The prefix of the symbols reserved for wildcards (`WildcardSymbol.RESERVED_PREFIX`): `*`.
    #[doc(alias = "RESERVED_PREFIX")]
    pub const WILDCARD_RESERVED_PREFIX: &'static str = "*";

    /// The indexed events of `symbol` from `source` (`new IndexedEventSubscriptionSymbol(symbol, source)`).
    pub fn indexed(symbol: impl Into<Symbol>, source: IndexedEventSource) -> Symbol {
        Symbol::IndexedEventSubscription {
            symbol: Box::new(symbol.into()),
            source,
        }
    }

    /// The time-series events of `symbol` from `from_time` on, in milliseconds since the Unix epoch
    /// (`new TimeSeriesSubscriptionSymbol(symbol, fromTime)`).
    pub fn time_series(symbol: impl Into<Symbol>, from_time: i64) -> Symbol {
        Symbol::TimeSeriesSubscription {
            symbol: Box::new(symbol.into()),
            from_time,
        }
    }

    /// The symbol of the events: the wrapped symbol of an indexed or a time-series subscription symbol, this symbol
    /// for the other kinds (`IndexedEventSubscriptionSymbol.getEventSymbol`,
    /// `TimeSeriesSubscriptionSymbol.getEventSymbol`).
    #[doc(alias = "getEventSymbol")]
    pub fn get_event_symbol(&self) -> &Symbol {
        match self {
            Symbol::IndexedEventSubscription { symbol, .. } | Symbol::TimeSeriesSubscription { symbol, .. } => symbol,
            _ => self,
        }
    }

    /// The source of an indexed event subscription symbol, `None` for the other kinds
    /// (`IndexedEventSubscriptionSymbol.getSource`).
    #[doc(alias = "getSource")]
    pub fn get_source(&self) -> Option<IndexedEventSource> {
        match self {
            Symbol::IndexedEventSubscription { source, .. } => Some(*source),
            _ => None,
        }
    }

    /// The time of the first event of a time-series subscription symbol, in milliseconds since the Unix epoch;
    /// `None` for the other kinds (`TimeSeriesSubscriptionSymbol.getFromTime`).
    #[doc(alias = "getFromTime")]
    pub fn get_from_time(&self) -> Option<i64> {
        match self {
            Symbol::TimeSeriesSubscription { from_time, .. } => Some(*from_time),
            _ => None,
        }
    }
}

impl Display for Symbol {
    /// Prints the symbol as Java's `toString` does: `AAPL`, `AAPL{=d}`, `*`, `AAPL{source=NTV}` or
    /// `AAPL{fromTime=20260101-000000.000}`.
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Symbol::String(symbol) => f.write_str(symbol),
            Symbol::Candle(symbol) => write!(f, "{symbol}"),
            Symbol::Wildcard => f.write_str("*"),
            Symbol::IndexedEventSubscription { symbol, source } => write!(f, "{symbol}{{source={source}}}"),
            Symbol::TimeSeriesSubscription { symbol, from_time } => {
                match TimeFormat::DEFAULT.with_millis().format(*from_time) {
                    Ok(time) => write!(f, "{symbol}{{fromTime={time}}}"),
                    Err(_) => write!(f, "{symbol}{{fromTime={from_time}}}"),
                }
            }
        }
    }
}

impl From<&str> for Symbol {
    fn from(symbol: &str) -> Self {
        Symbol::String(symbol.to_owned())
    }
}

impl From<&&str> for Symbol {
    fn from(symbol: &&str) -> Self {
        Symbol::String((*symbol).to_owned())
    }
}

impl From<String> for Symbol {
    fn from(symbol: String) -> Self {
        Symbol::String(symbol)
    }
}

impl From<&String> for Symbol {
    fn from(symbol: &String) -> Self {
        Symbol::String(symbol.clone())
    }
}

impl From<Cow<'_, str>> for Symbol {
    fn from(symbol: Cow<'_, str>) -> Self {
        Symbol::String(symbol.into_owned())
    }
}

impl From<&Cow<'_, str>> for Symbol {
    fn from(symbol: &Cow<'_, str>) -> Self {
        Symbol::String(symbol.as_ref().to_owned())
    }
}

impl From<CandleSymbol> for Symbol {
    fn from(symbol: CandleSymbol) -> Self {
        Symbol::Candle(symbol)
    }
}

impl From<&CandleSymbol> for Symbol {
    fn from(symbol: &CandleSymbol) -> Self {
        Symbol::Candle(symbol.clone())
    }
}

impl From<&Symbol> for Symbol {
    fn from(symbol: &Symbol) -> Self {
        symbol.clone()
    }
}

impl Symbol {
    /// The symbol of an SDK struct; `None` for NULL or a kind or source this crate does not know.
    ///
    /// # Safety
    ///
    /// `symbol` must be NULL or point to a live SDK symbol struct of the layout its `type_` names.
    pub(crate) unsafe fn from_sys(symbol: *const sys::dxfg_symbol_t) -> Option<Symbol> {
        // SAFETY: the caller guarantees that the struct is live and that its `type_` tells its layout; the nested
        // symbols and sources belong to it.
        unsafe {
            let kind = symbol.as_ref()?.type_;
            Some(match kind {
                sys::STRING => Symbol::String(text((*symbol.cast::<sys::dxfg_string_symbol_t>()).symbol)?),
                sys::CANDLE => Symbol::Candle(CandleSymbol::from_normalized(text(
                    (*symbol.cast::<sys::dxfg_candle_symbol_t>()).symbol,
                )?)),
                sys::WILDCARD => Symbol::Wildcard,
                sys::INDEXED_EVENT_SUBSCRIPTION => {
                    let indexed = &*symbol.cast::<sys::dxfg_indexed_event_subscription_symbol_t>();
                    Symbol::IndexedEventSubscription {
                        symbol: Box::new(Symbol::from_sys(indexed.symbol)?),
                        source: IndexedEventSource::from_sys(indexed.source.as_ref()?),
                    }
                }
                sys::TIME_SERIES_SUBSCRIPTION => {
                    let time_series = &*symbol.cast::<sys::dxfg_time_series_subscription_symbol_t>();
                    Symbol::TimeSeriesSubscription {
                        symbol: Box::new(Symbol::from_sys(time_series.symbol)?),
                        from_time: time_series.from_time,
                    }
                }
                _ => return None,
            })
        }
    }

    /// The symbols of an SDK list, without those [`from_sys`](Self::from_sys) does not know.
    ///
    /// # Safety
    ///
    /// `list` must be NULL or point to a live SDK list of live symbol structs.
    pub(crate) unsafe fn list_from_sys(list: *const sys::dxfg_symbol_list, component: &str) -> Vec<Symbol> {
        // SAFETY: the caller guarantees that the list and its `size` elements are live.
        unsafe {
            let Some(list) = list.as_ref() else {
                return Vec::new();
            };
            if list.elements.is_null() || list.size <= 0 {
                return Vec::new();
            }
            std::slice::from_raw_parts(list.elements, list.size as usize)
                .iter()
                .filter_map(|&symbol| {
                    let known = Symbol::from_sys(symbol);
                    if known.is_none() {
                        crate::ErrorManager::report(
                            component,
                            "skipped a symbol of a kind or source this crate does not know",
                        );
                    }
                    known
                })
                .collect()
        }
    }
}

/// A copy of an SDK string; `None` for NULL.
///
/// # Safety
///
/// `text` must be NULL or a live NUL-terminated string.
unsafe fn text(text: *const c_char) -> Option<String> {
    // SAFETY: the caller guarantees that a non-NULL string is live and NUL-terminated.
    (!text.is_null()).then(|| unsafe { isolate::string(text) }.into_owned())
}

/// The SDK structs of the symbols of one call, freed when the arena is dropped.
pub(crate) struct SymbolArena {
    strings: Vec<CString>,
    frees: Vec<Box<dyn FnOnce()>>,
}

impl SymbolArena {
    pub(crate) fn new() -> Self {
        SymbolArena {
            strings: Vec::new(),
            frees: Vec::new(),
        }
    }

    /// The SDK list of `symbols`; with `candle`, the string symbols are passed as candle symbols, see
    /// [`symbol`](Self::symbol).
    pub(crate) fn list(
        &mut self,
        symbols: impl IntoIterator<Item: Into<Symbol>>,
        candle: bool,
    ) -> Result<*mut sys::dxfg_symbol_list> {
        let mut pointers = symbols
            .into_iter()
            .map(|symbol| self.symbol(&symbol.into(), candle))
            .collect::<Result<Vec<_>>>()?;
        let list = sys::dxfg_symbol_list {
            size: pointers.len() as i32,
            elements: pointers.as_mut_ptr(),
        };
        let list = self.keep(list);
        self.keep(pointers);
        Ok(list)
    }

    /// The SDK struct of `symbol`; with `candle`, a string symbol is passed as a candle symbol, as the SDK needs for
    /// the requests for candles.
    pub(crate) fn symbol(&mut self, symbol: &Symbol, candle: bool) -> Result<*mut sys::dxfg_symbol_t> {
        Ok(match symbol {
            Symbol::String(text) if candle => self.candle(text)?,
            Symbol::String(text) => {
                let symbol = sys::dxfg_string_symbol_t {
                    supper: sys::dxfg_symbol_t { type_: sys::STRING },
                    symbol: self.string(text, "the symbol")?,
                };
                self.keep(symbol).cast()
            }
            Symbol::Candle(symbol) => self.candle(symbol.as_str())?,
            Symbol::Wildcard => self
                .keep(sys::dxfg_wildcard_symbol_t {
                    supper: sys::dxfg_symbol_t { type_: sys::WILDCARD },
                })
                .cast(),
            Symbol::IndexedEventSubscription { symbol, source } => {
                let symbol = sys::dxfg_indexed_event_subscription_symbol_t {
                    supper: sys::dxfg_symbol_t {
                        type_: sys::INDEXED_EVENT_SUBSCRIPTION,
                    },
                    symbol: self.symbol(symbol, candle)?,
                    source: self.source(*source),
                };
                self.keep(symbol).cast()
            }
            Symbol::TimeSeriesSubscription { symbol, from_time } => {
                let symbol = sys::dxfg_time_series_subscription_symbol_t {
                    supper: sys::dxfg_symbol_t {
                        type_: sys::TIME_SERIES_SUBSCRIPTION,
                    },
                    symbol: self.symbol(symbol, candle)?,
                    from_time: *from_time,
                };
                self.keep(symbol).cast()
            }
        })
    }

    /// The SDK list of `sources`, valid while the arena lives.
    pub(crate) fn source_list(
        &mut self,
        sources: impl IntoIterator<Item = IndexedEventSource>,
    ) -> *mut sys::dxfg_indexed_event_source_list {
        let mut pointers: Vec<_> = sources.into_iter().map(|source| self.source(source)).collect();
        let list = sys::dxfg_indexed_event_source_list {
            size: pointers.len() as i32,
            elements: pointers.as_mut_ptr(),
        };
        let list = self.keep(list);
        self.keep(pointers);
        list
    }

    /// The SDK struct of `source`.
    pub(crate) fn source(&mut self, source: IndexedEventSource) -> *mut sys::dxfg_indexed_event_source_t {
        // The SDK finds an `OrderSource` by its name, so a source of Java goes with the name of its id and any other,
        // made by `IndexedEventSource::new`, as Java's `new IndexedEventSource(id, name)`.
        let (type_, name) = match source.order_source_name() {
            Some(name) if source != IndexedEventSource::DEFAULT => (sys::ORDER_SOURCE, name),
            _ => (sys::INDEXED_EVENT_SOURCE, source.get_name()),
        };
        let source = sys::dxfg_indexed_event_source_t {
            type_,
            id: source.get_id(),
            // A source name has no NUL bytes.
            name: self.string(&name, "the source name").unwrap_or(c"".as_ptr()),
        };
        self.keep(source)
    }

    fn candle(&mut self, text: &str) -> Result<*mut sys::dxfg_symbol_t> {
        let symbol = sys::dxfg_candle_symbol_t {
            supper: sys::dxfg_symbol_t { type_: sys::CANDLE },
            symbol: self.string(text, "the candle symbol")?,
        };
        Ok(self.keep(symbol).cast())
    }

    /// A NUL-terminated copy of `text`, valid while the arena lives.
    pub(crate) fn string(&mut self, text: &str, what: &str) -> Result<*const std::ffi::c_char> {
        self.strings.push(c_string(text, what)?);
        // The pointer is that of the heap buffer of the string, which does not move with it.
        Ok(self.strings.last().map_or(std::ptr::null(), |s| s.as_ptr()))
    }

    /// Moves `value` to the heap until the arena is dropped.
    fn keep<T: 'static>(&mut self, value: T) -> *mut T {
        let raw = Box::into_raw(Box::new(value));
        // SAFETY: `raw` came from `Box::into_raw` and is freed once, when the arena is dropped.
        self.frees.push(Box::new(move || drop(unsafe { Box::from_raw(raw) })));
        raw
    }
}

impl Drop for SymbolArena {
    fn drop(&mut self) {
        for free in self.frees.drain(..) {
            free();
        }
    }
}
