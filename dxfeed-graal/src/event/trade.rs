//! Trade events.

use std::borrow::Cow;
use std::ffi::c_char;

use dxfeed_graal_sys as sys;

use super::format::{JavaChar, JavaDouble, Time, direction, extended_trading_hours, year_month_day};
use super::{MILLIS_MASK, SEQUENCE_BITS, SEQUENCE_MASK, exchange_code, market_event, sys_exchange_code};
use crate::error::Result;
use crate::isolate::string;

/// A trade received by a listener: the last trade of a symbol (`Trade`). Valid during the callback only.
#[repr(transparent)]
pub struct TradeRef(sys::dxfg_trade_t);

impl TradeRef {
    fn base(&self) -> &sys::dxfg_trade_base_t {
        &self.0.trade_base
    }

    /// The symbol; replacement characters stand for bytes that are not UTF-8.
    pub fn symbol(&self) -> Cow<'_, str> {
        // SAFETY: the SDK keeps the symbol alive while the event lives.
        unsafe { string(self.base().market_event.event_symbol) }
    }

    /// The time the event was created, in milliseconds since the Unix epoch; `0` if unknown.
    pub fn event_time(&self) -> i64 {
        self.base().market_event.event_time
    }

    /// The time and the sequence of the trade, packed as the SDK keeps them.
    pub fn time_sequence(&self) -> i64 {
        self.base().time_sequence
    }

    /// The nanoseconds part of the trade time.
    pub fn time_nano_part(&self) -> i32 {
        self.base().time_nano_part
    }

    /// The exchange code of the trade.
    pub fn exchange_code(&self) -> char {
        exchange_code(self.base().exchange_code)
    }

    /// The price; NaN if unknown.
    pub fn price(&self) -> f64 {
        self.base().price
    }

    /// The change of the price from the previous trade; NaN if unknown.
    pub fn change(&self) -> f64 {
        self.base().change
    }

    /// The size; NaN if unknown.
    pub fn size(&self) -> f64 {
        self.base().size
    }

    /// The identifier of the trading day.
    pub fn day_id(&self) -> i32 {
        self.base().day_id
    }

    /// The volume traded during the day; NaN if unknown.
    pub fn day_volume(&self) -> f64 {
        self.base().day_volume
    }

    /// The turnover during the day; NaN if unknown.
    pub fn day_turnover(&self) -> f64 {
        self.base().day_turnover
    }

    /// The flags of the trade (tick direction, extended trading hours), as the SDK keeps them.
    pub fn flags(&self) -> i32 {
        self.base().flags
    }

    /// The identifier of the trade.
    pub fn trade_id(&self) -> i64 {
        self.base().trade_id
    }

    /// Copies the trade.
    pub fn to_owned(&self) -> Trade {
        Trade {
            symbol: self.symbol().into_owned(),
            event_time: self.event_time(),
            time_sequence: self.time_sequence(),
            time_nano_part: self.time_nano_part(),
            exchange_code: self.exchange_code(),
            price: self.price(),
            change: self.change(),
            size: self.size(),
            day_id: self.day_id(),
            day_volume: self.day_volume(),
            day_turnover: self.day_turnover(),
            flags: self.flags(),
            trade_id: self.trade_id(),
        }
    }
}

impl std::fmt::Debug for TradeRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.to_owned().fmt(f)
    }
}

/// The text of Java's `Trade.toString`, e.g. `Trade{AAPL, eventTime=0, time=20241004-093000.000, ...}`.
impl std::fmt::Display for TradeRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.to_owned().fmt(f)
    }
}

/// An owned trade; see [`TradeRef`] for the fields.
#[derive(Debug, Clone, PartialEq)]
pub struct Trade {
    /// The symbol.
    pub symbol: String,
    /// The time the event was created, in milliseconds since the Unix epoch; `0` if unknown.
    pub event_time: i64,
    /// The time and the sequence of the trade, packed as the SDK keeps them.
    pub time_sequence: i64,
    /// The nanoseconds part of the trade time.
    pub time_nano_part: i32,
    /// The exchange code of the trade; it must fit a Java `char`.
    pub exchange_code: char,
    /// The price; NaN if unknown.
    pub price: f64,
    /// The change of the price from the previous trade; NaN if unknown.
    pub change: f64,
    /// The size; NaN if unknown.
    pub size: f64,
    /// The identifier of the trading day.
    pub day_id: i32,
    /// The volume traded during the day; NaN if unknown.
    pub day_volume: f64,
    /// The turnover during the day; NaN if unknown.
    pub day_turnover: f64,
    /// The flags of the trade, as the SDK keeps them.
    pub flags: i32,
    /// The identifier of the trade.
    pub trade_id: i64,
}

impl Trade {
    /// A trade for `symbol` with unknown prices, sizes and volumes (NaN) and zero times, as Java creates it.
    pub fn new(symbol: &str) -> Self {
        Trade {
            symbol: symbol.to_owned(),
            event_time: 0,
            time_sequence: 0,
            time_nano_part: 0,
            exchange_code: '\0',
            price: f64::NAN,
            change: f64::NAN,
            size: f64::NAN,
            day_id: 0,
            day_volume: f64::NAN,
            day_turnover: f64::NAN,
            flags: 0,
            trade_id: 0,
        }
    }

    /// The trade time in milliseconds, as Java's `TradeBase.getTime` computes it.
    fn time(&self) -> i64 {
        (self.time_sequence >> 32) * 1000 + ((self.time_sequence >> SEQUENCE_BITS) & MILLIS_MASK)
    }

    /// The sequence of the trade, as Java's `TradeBase.getSequence` computes it.
    fn sequence(&self) -> i32 {
        self.time_sequence as i32 & SEQUENCE_MASK
    }

    pub(super) fn to_sys(&self, symbol: *const c_char) -> Result<sys::dxfg_trade_t> {
        Ok(sys::dxfg_trade_t {
            trade_base: sys::dxfg_trade_base_t {
                market_event: market_event(sys::DXFG_EVENT_TRADE, symbol, self.event_time),
                time_sequence: self.time_sequence,
                time_nano_part: self.time_nano_part,
                exchange_code: sys_exchange_code(self.exchange_code)?,
                price: self.price,
                change: self.change,
                size: self.size,
                day_id: self.day_id,
                day_volume: self.day_volume,
                day_turnover: self.day_turnover,
                flags: self.flags,
                trade_id: self.trade_id,
            },
        })
    }
}

/// The text of Java's `Trade.toString`; times print in the default time zone of the SDK.
impl std::fmt::Display for Trade {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Trade{{{}, eventTime={}, time={}, timeNanoPart={}, sequence={}, exchange={}, price={}, change={}, \
             size={}, day={}, dayVolume={}, dayTurnover={}, direction={}, ETH={}}}",
            self.symbol,
            Time::millis(self.event_time),
            Time::millis(self.time()),
            self.time_nano_part,
            self.sequence(),
            JavaChar(self.exchange_code),
            JavaDouble(self.price),
            JavaDouble(self.change),
            JavaDouble(self.size),
            year_month_day(self.day_id),
            JavaDouble(self.day_volume),
            JavaDouble(self.day_turnover),
            direction(self.flags),
            extended_trading_hours(self.flags),
        )
    }
}
