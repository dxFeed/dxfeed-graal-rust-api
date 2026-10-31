//! Market maker events.

use dxfeed_graal_sys as sys;

use super::flags::{MARKET_MAKER_EXCHANGE, MARKET_MAKER_ID, decode_short_string, encode_short_string};
use super::format::{JavaChar, JavaDouble, Quoted, Time};

event_type! {
    head {
        /// The best bid and ask of a market maker on an exchange (`MarketMaker`).
        MarketMaker, MarketMakerRef, sys::dxfg_market_maker_t, sys::DXFG_EVENT_MARKET_MAKER, [market_event]
    }
    fields {
        /// The flags of the event in a transaction or a snapshot.
        event_flags: event_flags [event_flags] = super::EventFlags::empty(),
        /// The index of the event: the exchange code in bits 32 to 47 and the market maker identifier in bits 0 to
        /// 31, as [`exchange_code`](Self::exchange_code) and [`market_maker`](Self::market_maker) read them.
        index: i64 [index] = 0,
        /// The time of the bid, in milliseconds since the Unix epoch.
        bid_time: i64 [bid_time] = 0,
        /// The bid price; NaN if unknown.
        bid_price: f64 [bid_price] = f64::NAN,
        /// The bid size; NaN if unknown.
        bid_size: f64 [bid_size] = f64::NAN,
        /// The number of the bid orders.
        bid_count: i64 [bid_count] = 0,
        /// The time of the ask, in milliseconds since the Unix epoch.
        ask_time: i64 [ask_time] = 0,
        /// The ask price; NaN if unknown.
        ask_price: f64 [ask_price] = f64::NAN,
        /// The ask size; NaN if unknown.
        ask_size: f64 [ask_size] = f64::NAN,
        /// The number of the ask orders.
        ask_count: i64 [ask_count] = 0,
    }
    derived {
        /// The source of the event: always the default source `DEFAULT`, as Java's `getSource` gives it.
        #[doc(alias = "getSource")]
        fn source() -> super::IndexedEventSource {
            super::IndexedEventSource::DEFAULT
        }

        /// The time of the event: the later of [`bid_time`](Self::bid_time) and [`ask_time`](Self::ask_time).
        #[doc(alias = "getTime")]
        fn time(bid_time, ask_time) -> i64 {
            bid_time.max(ask_time)
        }

        /// The exchange code of the market maker; `'\0'` if unknown.
        #[doc(alias = "getExchangeCode")]
        fn exchange_code(index) -> char {
            super::exchange_code(MARKET_MAKER_EXCHANGE.get(index) as i16)
        }

        /// The identifier of the market maker, up to 4 ASCII characters; `None` if unknown.
        #[doc(alias = "getMarketMaker")]
        fn market_maker(index) -> Option<String> {
            decode_short_string(MARKET_MAKER_ID.get(index))
        }
    }
}

impl MarketMaker {
    /// Changes the exchange code of the market maker in the [`index`](Self::index).
    ///
    /// # Errors
    ///
    /// [`Error::InvalidArgument`](crate::Error::InvalidArgument) if the code does not fit a Java `char`.
    #[doc(alias = "setExchangeCode")]
    pub fn set_exchange_code(&mut self, code: char) -> crate::Result<()> {
        let code = super::sys_exchange_code(code)?;
        self.index = MARKET_MAKER_EXCHANGE.set(self.index, i32::from(code as u16));
        Ok(())
    }

    /// Changes the identifier of the market maker in the [`index`](Self::index).
    ///
    /// # Errors
    ///
    /// [`Error::InvalidArgument`](crate::Error::InvalidArgument) if the identifier is longer than 4 ASCII characters
    /// or has a NUL.
    #[doc(alias = "setMarketMaker")]
    pub fn set_market_maker(&mut self, market_maker: &str) -> crate::Result<()> {
        self.index = MARKET_MAKER_ID.set(self.index, encode_short_string(market_maker)?);
        Ok(())
    }
}

/// The text of Java's `MarketMaker.toString`, e.g. `MarketMaker{AAPL, eventTime=0, source=DEFAULT, ...}`.
impl std::fmt::Display for MarketMaker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "MarketMaker{{{}, eventTime={}, source=DEFAULT, eventFlags=0x{:x}, index=0x{:x}, time={}, exchange={}, \
             marketMaker={}, bidTime={}, bidPrice={}, bidSize={}, bidCount={}, askTime={}, askPrice={}, askSize={}, \
             askCount={}}}",
            self.symbol,
            Time(self.event_time),
            self.event_flags,
            self.index,
            Time(self.time()),
            JavaChar(self.exchange_code()),
            Quoted(self.market_maker().as_deref()),
            Time(self.bid_time),
            JavaDouble(self.bid_price),
            JavaDouble(self.bid_size),
            self.bid_count,
            Time(self.ask_time),
            JavaDouble(self.ask_price),
            JavaDouble(self.ask_size),
            self.ask_count,
        )
    }
}
