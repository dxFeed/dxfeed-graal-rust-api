//! Summary events.

use dxfeed_graal_sys as sys;

use super::flags::{PriceType, SUMMARY_DAY_CLOSE_TYPE, SUMMARY_PREV_DAY_CLOSE_TYPE};
use super::format::{JavaDouble, Time, year_month_day};

event_type! {
    head {
        /// A summary: the open, high, low and close prices of the day and of the previous day (`Summary`).
        Summary, SummaryRef, sys::dxfg_summary_t, sys::DXFG_EVENT_SUMMARY, [market_event]
    }
    fields {
        /// The identifier of the day, in days since 1970-01-01.
        #[doc(alias = "getDayId")]
        day_id: i32 [day_id] = 0,
        /// The first price of the day; NaN if unknown.
        #[doc(alias = "getDayOpenPrice")]
        day_open_price: f64 [day_open_price] = f64::NAN,
        /// The highest price of the day; NaN if unknown.
        #[doc(alias = "getDayHighPrice")]
        day_high_price: f64 [day_high_price] = f64::NAN,
        /// The lowest price of the day; NaN if unknown.
        #[doc(alias = "getDayLowPrice")]
        day_low_price: f64 [day_low_price] = f64::NAN,
        /// The last price of the day; NaN if unknown.
        #[doc(alias = "getDayClosePrice")]
        day_close_price: f64 [day_close_price] = f64::NAN,
        /// The identifier of the previous day, in days since 1970-01-01.
        #[doc(alias = "getPrevDayId")]
        prev_day_id: i32 [prev_day_id] = 0,
        /// The last price of the previous day; NaN if unknown.
        #[doc(alias = "getPrevDayClosePrice")]
        prev_day_close_price: f64 [prev_day_close_price] = f64::NAN,
        /// The volume of the previous day; NaN if unknown.
        #[doc(alias = "getPrevDayVolume")]
        prev_day_volume: f64 [prev_day_volume] = f64::NAN,
        /// The open interest of the symbol.
        #[doc(alias = "getOpenInterest")]
        open_interest: i64 [open_interest] = 0,
        /// The flags of the summary, packed as the SDK keeps them: the type of the day close price in bits 2 and 3,
        /// the type of the previous day close price in bits 0 and 1.
        flags: i32 [flags] = 0,
    }
    derived {
        /// The type of the day close price.
        #[doc(alias = "getDayClosePriceType")]
        fn day_close_price_type(flags) -> PriceType {
            PriceType::from_code(SUMMARY_DAY_CLOSE_TYPE.get32(flags))
        }

        /// The type of the previous day close price.
        #[doc(alias = "getPrevDayClosePriceType")]
        fn prev_day_close_price_type(flags) -> PriceType {
            PriceType::from_code(SUMMARY_PREV_DAY_CLOSE_TYPE.get32(flags))
        }
    }
}

impl Summary {
    /// Changes the type of the day close price.
    #[doc(alias = "setDayClosePriceType")]
    pub fn set_day_close_price_type(&mut self, price_type: PriceType) {
        self.flags = SUMMARY_DAY_CLOSE_TYPE.set32(self.flags, price_type.get_code());
    }

    /// Changes the type of the previous day close price.
    #[doc(alias = "setPrevDayClosePriceType")]
    pub fn set_prev_day_close_price_type(&mut self, price_type: PriceType) {
        self.flags = SUMMARY_PREV_DAY_CLOSE_TYPE.set32(self.flags, price_type.get_code());
    }
}

/// The text of Java's `Summary.toString`, e.g. `Summary{AAPL, eventTime=0, day=20241004, dayOpen=226.5, ...}`.
impl std::fmt::Display for Summary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Summary{{{}, eventTime={}, day={}, dayOpen={}, dayHigh={}, dayLow={}, dayClose={}, dayCloseType={}, \
             prevDay={}, prevDayClose={}, prevDayCloseType={}, prevDayVolume={}, openInterest={}}}",
            self.symbol,
            Time(self.event_time),
            year_month_day(self.day_id),
            JavaDouble(self.day_open_price),
            JavaDouble(self.day_high_price),
            JavaDouble(self.day_low_price),
            JavaDouble(self.day_close_price),
            self.day_close_price_type(),
            year_month_day(self.prev_day_id),
            JavaDouble(self.prev_day_close_price),
            self.prev_day_close_price_type(),
            JavaDouble(self.prev_day_volume),
            self.open_interest,
        )
    }
}
