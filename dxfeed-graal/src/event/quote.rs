//! Quote events.

use std::borrow::Cow;
use std::ffi::c_char;

use dxfeed_graal_sys as sys;

use super::format::{JavaChar, JavaDouble, Time};
use super::{SEQUENCE_BITS, SEQUENCE_MASK, exchange_code, market_event, sys_exchange_code};
use crate::error::Result;
use crate::isolate::string;

/// A quote received by a listener: the best bid and offer of a symbol (`Quote`). Valid during the callback only.
#[repr(transparent)]
pub struct QuoteRef(sys::dxfg_quote_t);

impl QuoteRef {
    /// The symbol; replacement characters stand for bytes that are not UTF-8.
    pub fn symbol(&self) -> Cow<'_, str> {
        // SAFETY: the SDK keeps the symbol alive while the event lives.
        unsafe { string(self.0.market_event.event_symbol) }
    }

    /// The time the event was created, in milliseconds since the Unix epoch; `0` if unknown.
    pub fn event_time(&self) -> i64 {
        self.0.market_event.event_time
    }

    /// The milliseconds and the sequence of the quote time, packed as the SDK keeps them.
    pub fn time_millis_sequence(&self) -> i32 {
        self.0.time_millis_sequence
    }

    /// The nanoseconds part of the quote time.
    pub fn time_nano_part(&self) -> i32 {
        self.0.time_nano_part
    }

    /// The time of the last bid change, in milliseconds since the Unix epoch.
    pub fn bid_time(&self) -> i64 {
        self.0.bid_time
    }

    /// The exchange code of the bid.
    pub fn bid_exchange_code(&self) -> char {
        exchange_code(self.0.bid_exchange_code)
    }

    /// The bid price; NaN if unknown.
    pub fn bid_price(&self) -> f64 {
        self.0.bid_price
    }

    /// The bid size; NaN if unknown.
    pub fn bid_size(&self) -> f64 {
        self.0.bid_size
    }

    /// The time of the last ask change, in milliseconds since the Unix epoch.
    pub fn ask_time(&self) -> i64 {
        self.0.ask_time
    }

    /// The exchange code of the ask.
    pub fn ask_exchange_code(&self) -> char {
        exchange_code(self.0.ask_exchange_code)
    }

    /// The ask price; NaN if unknown.
    pub fn ask_price(&self) -> f64 {
        self.0.ask_price
    }

    /// The ask size; NaN if unknown.
    pub fn ask_size(&self) -> f64 {
        self.0.ask_size
    }

    /// Copies the quote.
    pub fn to_owned(&self) -> Quote {
        Quote {
            symbol: self.symbol().into_owned(),
            event_time: self.event_time(),
            time_millis_sequence: self.time_millis_sequence(),
            time_nano_part: self.time_nano_part(),
            bid_time: self.bid_time(),
            bid_exchange_code: self.bid_exchange_code(),
            bid_price: self.bid_price(),
            bid_size: self.bid_size(),
            ask_time: self.ask_time(),
            ask_exchange_code: self.ask_exchange_code(),
            ask_price: self.ask_price(),
            ask_size: self.ask_size(),
        }
    }
}

impl std::fmt::Debug for QuoteRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.to_owned().fmt(f)
    }
}

/// The text of Java's `Quote.toString`, e.g. `Quote{AAPL, eventTime=0, time=20241004-093000.000, ...}`.
impl std::fmt::Display for QuoteRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.to_owned().fmt(f)
    }
}

/// An owned quote; see [`QuoteRef`] for the fields.
#[derive(Debug, Clone, PartialEq)]
pub struct Quote {
    /// The symbol.
    pub symbol: String,
    /// The time the event was created, in milliseconds since the Unix epoch; `0` if unknown.
    pub event_time: i64,
    /// The milliseconds and the sequence of the quote time, packed as the SDK keeps them.
    pub time_millis_sequence: i32,
    /// The nanoseconds part of the quote time.
    pub time_nano_part: i32,
    /// The time of the last bid change, in milliseconds since the Unix epoch.
    pub bid_time: i64,
    /// The exchange code of the bid; it must fit a Java `char`.
    pub bid_exchange_code: char,
    /// The bid price; NaN if unknown.
    pub bid_price: f64,
    /// The bid size; NaN if unknown.
    pub bid_size: f64,
    /// The time of the last ask change, in milliseconds since the Unix epoch.
    pub ask_time: i64,
    /// The exchange code of the ask; it must fit a Java `char`.
    pub ask_exchange_code: char,
    /// The ask price; NaN if unknown.
    pub ask_price: f64,
    /// The ask size; NaN if unknown.
    pub ask_size: f64,
}

impl Quote {
    /// A quote for `symbol` with unknown prices and sizes (NaN) and zero times, as Java creates it.
    pub fn new(symbol: &str) -> Self {
        Quote {
            symbol: symbol.to_owned(),
            event_time: 0,
            time_millis_sequence: 0,
            time_nano_part: 0,
            bid_time: 0,
            bid_exchange_code: '\0',
            bid_price: f64::NAN,
            bid_size: f64::NAN,
            ask_time: 0,
            ask_exchange_code: '\0',
            ask_price: f64::NAN,
            ask_size: f64::NAN,
        }
    }

    /// The quote time in milliseconds, as Java's `Quote.getTime` computes it.
    fn time(&self) -> i64 {
        self.bid_time.max(self.ask_time).div_euclid(1000) * 1000
            + i64::from((self.time_millis_sequence as u32) >> SEQUENCE_BITS)
    }

    /// The sequence of the quote, as Java's `Quote.getSequence` computes it.
    fn sequence(&self) -> i32 {
        self.time_millis_sequence & SEQUENCE_MASK
    }

    pub(super) fn to_sys(&self, symbol: *const c_char) -> Result<sys::dxfg_quote_t> {
        Ok(sys::dxfg_quote_t {
            market_event: market_event(sys::DXFG_EVENT_QUOTE, symbol, self.event_time),
            time_millis_sequence: self.time_millis_sequence,
            time_nano_part: self.time_nano_part,
            bid_time: self.bid_time,
            bid_exchange_code: sys_exchange_code(self.bid_exchange_code)?,
            bid_price: self.bid_price,
            bid_size: self.bid_size,
            ask_time: self.ask_time,
            ask_exchange_code: sys_exchange_code(self.ask_exchange_code)?,
            ask_price: self.ask_price,
            ask_size: self.ask_size,
        })
    }
}

/// The text of Java's `Quote.toString`; times print in the default time zone of the SDK.
impl std::fmt::Display for Quote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Quote{{{}, eventTime={}, time={}, timeNanoPart={}, sequence={}, bidTime={}, bidExchange={}, bidPrice={}, \
             bidSize={}, askTime={}, askExchange={}, askPrice={}, askSize={}}}",
            self.symbol,
            Time::millis(self.event_time),
            Time::millis(self.time()),
            self.time_nano_part,
            self.sequence(),
            Time::seconds(self.bid_time),
            JavaChar(self.bid_exchange_code),
            JavaDouble(self.bid_price),
            JavaDouble(self.bid_size),
            Time::seconds(self.ask_time),
            JavaChar(self.ask_exchange_code),
            JavaDouble(self.ask_price),
            JavaDouble(self.ask_size),
        )
    }
}
