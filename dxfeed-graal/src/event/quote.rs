//! Quote events.

use dxfeed_graal_sys as sys;

use super::flags;
use super::format::{JavaChar, JavaDouble, Time};
use crate::error::Result;

event_type! {
    head {
        /// A quote: the best bid and offer of a symbol (`Quote`).
        ///
        /// # Examples
        ///
        /// ```no_run
        /// # fn main() -> dxfeed_graal::Result<()> {
        /// use dxfeed_graal::DXFeed;
        /// use dxfeed_graal::event::Quote;
        ///
        /// let subscription = DXFeed::get_instance()?.create_subscription::<Quote>()?;
        /// subscription.add_event_listener(|quotes| {
        ///     for q in quotes {
        ///         let (bid, ask) = (q.bid_price(), q.ask_price());
        ///         println!("{}: {bid} x {} / {ask} x {}", q.symbol(), q.bid_size(), q.ask_size());
        ///         println!("spread {}", ask - bid);
        ///     }
        /// })?;
        /// subscription.add_symbols(["AAPL"])?;
        /// # Ok(())
        /// # }
        /// ```
        Quote, QuoteRef, sys::dxfg_quote_t, sys::DXFG_EVENT_QUOTE, [market_event]
    }
    fields {
        /// The milliseconds and the sequence of the quote time, packed as the SDK keeps them: milliseconds in bits 22
        /// to 31, the sequence in bits 0 to 21.
        time_millis_sequence: i32 [time_millis_sequence] = 0,
        /// The microseconds and nanoseconds part of the quote time.
        #[doc(alias = "getTimeNanoPart")]
        time_nano_part: i32 [time_nano_part] = 0,
        /// The time of the last bid change, in milliseconds since the Unix epoch.
        #[doc(alias = "getBidTime")]
        bid_time: i64 [bid_time] = 0,
        /// The exchange code of the bid; it must fit a Java `char`.
        #[doc(alias = "getBidExchangeCode")]
        bid_exchange_code: char [bid_exchange_code] = '\0',
        /// The bid price; NaN if unknown.
        #[doc(alias = "getBidPrice")]
        bid_price: f64 [bid_price] = f64::NAN,
        /// The bid size; NaN if unknown.
        #[doc(alias = "getBidSizeAsDouble")]
        bid_size: f64 [bid_size] = f64::NAN,
        /// The time of the last ask change, in milliseconds since the Unix epoch.
        #[doc(alias = "getAskTime")]
        ask_time: i64 [ask_time] = 0,
        /// The exchange code of the ask; it must fit a Java `char`.
        #[doc(alias = "getAskExchangeCode")]
        ask_exchange_code: char [ask_exchange_code] = '\0',
        /// The ask price; NaN if unknown.
        #[doc(alias = "getAskPrice")]
        ask_price: f64 [ask_price] = f64::NAN,
        /// The ask size; NaN if unknown.
        #[doc(alias = "getAskSizeAsDouble")]
        ask_size: f64 [ask_size] = f64::NAN,
    }
    derived {
        /// The bid size as a whole number, as Java's `getBidSize` gives it:
        /// the fraction is dropped, NaN gives `0` and the values beyond `i64` saturate.
        #[doc(alias = "getBidSize")]
        fn bid_size_as_long(bid_size) -> i64 {
            bid_size as i64
        }

        /// The ask size as a whole number, as Java's `getAskSize` gives it:
        /// the fraction is dropped, NaN gives `0` and the values beyond `i64` saturate.
        #[doc(alias = "getAskSize")]
        fn ask_size_as_long(ask_size) -> i64 {
            ask_size as i64
        }

        /// The time of the last bid or ask change, in milliseconds since the Unix epoch.
        #[doc(alias = "getTime")]
        fn time(bid_time, ask_time, time_millis_sequence) -> i64 {
            quote_time(bid_time, ask_time, time_millis_sequence)
        }

        /// The time of the last bid or ask change, in nanoseconds since the Unix epoch.
        #[doc(alias = "getTimeNanos")]
        fn time_nanos(bid_time, ask_time, time_millis_sequence, time_nano_part) -> i64 {
            flags::nanos(quote_time(bid_time, ask_time, time_millis_sequence), time_nano_part)
        }

        /// The sequence number of the quote, to tell apart quotes with the same time.
        #[doc(alias = "getSequence")]
        fn sequence(time_millis_sequence) -> i32 {
            flags::sequence_of(time_millis_sequence.into())
        }
    }
}

/// The quote time as Java's `Quote.getTime` computes it, wrapping as Java's `long` does.
fn quote_time(bid_time: i64, ask_time: i64, time_millis_sequence: i32) -> i64 {
    bid_time
        .max(ask_time)
        .div_euclid(1000)
        .wrapping_mul(1000)
        .wrapping_add(i64::from(flags::MILLIS.get32(time_millis_sequence)))
}

impl Quote {
    /// The largest sequence of the quote, `(1 << 22) - 1`.
    pub const MAX_SEQUENCE: i32 = flags::MAX_SEQUENCE;

    /// Changes the time of the last bid change and the milliseconds of the quote time, as Java's `setBidTime` does.
    #[doc(alias = "setBidTime")]
    pub fn set_bid_time(&mut self, bid_time: i64) {
        self.bid_time = bid_time;
        self.recompute_time_millis();
    }

    /// Changes the time of the last ask change and the milliseconds of the quote time, as Java's `setAskTime` does.
    #[doc(alias = "setAskTime")]
    pub fn set_ask_time(&mut self, ask_time: i64) {
        self.ask_time = ask_time;
        self.recompute_time_millis();
    }

    /// Changes the sequence of the quote.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidArgument`](crate::Error::InvalidArgument) if the sequence is negative or above
    /// [`MAX_SEQUENCE`](Self::MAX_SEQUENCE).
    #[doc(alias = "setSequence")]
    pub fn set_sequence(&mut self, sequence: i32) -> Result<()> {
        self.time_millis_sequence = flags::with_sequence(self.time_millis_sequence.into(), sequence)? as i32;
        Ok(())
    }

    fn recompute_time_millis(&mut self) {
        let millis = self.bid_time.max(self.ask_time).rem_euclid(1000) as i32;
        self.time_millis_sequence = flags::MILLIS.set32(self.time_millis_sequence, millis);
    }
}

/// The text of Java's `Quote.toString`, e.g. `Quote{AAPL, eventTime=0, time=20241004-093000.000, ...}`; times print
/// in the default time zone of the SDK.
impl std::fmt::Display for Quote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Quote{{{}, eventTime={}, time={}, timeNanoPart={}, sequence={}, bidTime={}, bidExchange={}, bidPrice={}, \
             bidSize={}, askTime={}, askExchange={}, askPrice={}, askSize={}}}",
            self.symbol,
            Time(self.event_time),
            Time(self.time()),
            self.time_nano_part,
            self.sequence(),
            Time(self.bid_time),
            JavaChar(self.bid_exchange_code),
            JavaDouble(self.bid_price),
            JavaDouble(self.bid_size),
            Time(self.ask_time),
            JavaChar(self.ask_exchange_code),
            JavaDouble(self.ask_price),
            JavaDouble(self.ask_size),
        )
    }
}
