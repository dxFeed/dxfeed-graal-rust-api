//! The symbols of subscriptions (`WildcardSymbol`, `IndexedEventSubscriptionSymbol`, `TimeSeriesSubscriptionSymbol`).

use std::ffi::CString;
use std::fmt::{self, Display, Formatter};

use dxfeed_graal_sys as sys;

use crate::TimeFormat;
use crate::error::{Result, c_string};
use crate::event::{CandleSymbol, IndexedEventSource};

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
/// ```no_run
/// # fn main() -> dxfeed_graal::Result<()> {
/// use dxfeed_graal::event::{Candle, CandlePeriod, CandleSymbol};
/// use dxfeed_graal::{DXEndpoint, Symbol, TimeFormat};
///
/// let subscription = DXEndpoint::create()?.feed().create_subscription::<Candle>()?;
/// let symbol = CandleSymbol::value_of_with("AAPL", [CandlePeriod::DAY])?;
/// subscription.add_symbols([Symbol::time_series(symbol, TimeFormat::DEFAULT.parse("20260101")?)])?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq)]
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

    /// The SDK list of `symbols`.
    pub(crate) fn list(
        &mut self,
        symbols: impl IntoIterator<Item: Into<Symbol>>,
    ) -> Result<*mut sys::dxfg_symbol_list> {
        let mut pointers = symbols
            .into_iter()
            .map(|symbol| self.symbol(&symbol.into(), false))
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

    /// The SDK struct of `source`.
    pub(crate) fn source(&mut self, source: IndexedEventSource) -> *mut sys::dxfg_indexed_event_source_t {
        let name = source.name();
        let source = sys::dxfg_indexed_event_source_t {
            type_: if source == IndexedEventSource::DEFAULT {
                sys::INDEXED_EVENT_SOURCE
            } else {
                sys::ORDER_SOURCE
            },
            id: source.id(),
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
    fn string(&mut self, text: &str, what: &str) -> Result<*const std::ffi::c_char> {
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
