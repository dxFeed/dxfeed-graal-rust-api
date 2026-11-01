//! Profile events.

use dxfeed_graal_sys as sys;

use super::flags::{PROFILE_SSR, PROFILE_STATUS, ShortSaleRestriction, TradingStatus};
use super::format::{JavaDouble, Quoted, Time, TimeSeconds, WideDecimal, year_month_day};

event_type! {
    head {
        /// A profile: the description of a symbol, its trading status and limits (`Profile`).
        Profile, ProfileRef, sys::dxfg_profile_t, sys::DXFG_EVENT_PROFILE, [market_event]
    }
    fields {
        /// The description of the security.
        #[doc(alias = "getDescription")]
        description: string [description] = None,
        /// The reason of the trading halt or of the status change.
        #[doc(alias = "getStatusReason")]
        status_reason: string [status_reason] = None,
        /// The start of the trading halt, in milliseconds since the Unix epoch.
        #[doc(alias = "getHaltStartTime")]
        halt_start_time: i64 [halt_start_time] = 0,
        /// The end of the trading halt, in milliseconds since the Unix epoch.
        #[doc(alias = "getHaltEndTime")]
        halt_end_time: i64 [halt_end_time] = 0,
        /// The highest price allowed for the day; NaN if unknown.
        #[doc(alias = "getHighLimitPrice")]
        high_limit_price: f64 [high_limit_price] = f64::NAN,
        /// The lowest price allowed for the day; NaN if unknown.
        #[doc(alias = "getLowLimitPrice")]
        low_limit_price: f64 [low_limit_price] = f64::NAN,
        /// The highest price of the last 52 weeks; NaN if unknown.
        #[doc(alias = "getHigh52WeekPrice")]
        high_52_week_price: f64 [high_52_week_price] = f64::NAN,
        /// The lowest price of the last 52 weeks; NaN if unknown.
        #[doc(alias = "getLow52WeekPrice")]
        low_52_week_price: f64 [low_52_week_price] = f64::NAN,
        /// The beta of the security; NaN if unknown.
        #[doc(alias = "getBeta")]
        beta: f64 [beta] = f64::NAN,
        /// The earnings per share; NaN if unknown.
        #[doc(alias = "getEarningsPerShare")]
        earnings_per_share: f64 [earnings_per_share] = f64::NAN,
        /// The number of dividend payments a year; NaN if unknown.
        #[doc(alias = "getDividendFrequency")]
        dividend_frequency: f64 [dividend_frequency] = f64::NAN,
        /// The amount of the last dividend; NaN if unknown.
        #[doc(alias = "getExDividendAmount")]
        ex_dividend_amount: f64 [ex_dividend_amount] = f64::NAN,
        /// The day of the last dividend, in days since 1970-01-01.
        #[doc(alias = "getExDividendDayId")]
        ex_dividend_day_id: i32 [ex_dividend_day_id] = 0,
        /// The number of shares outstanding; NaN if unknown.
        #[doc(alias = "getShares")]
        shares: f64 [shares] = f64::NAN,
        /// The number of shares free to trade; NaN if unknown.
        #[doc(alias = "getFreeFloat")]
        free_float: f64 [free_float] = f64::NAN,
        /// The flags of the profile, packed as the SDK keeps them: the short sale restriction in bits 2 and 3, the
        /// trading status in bits 0 and 1.
        flags: i32 [flags] = 0,
    }
    derived {
        /// The short sale restriction of the security.
        #[doc(alias = "getShortSaleRestriction")]
        fn short_sale_restriction(flags) -> ShortSaleRestriction {
            ShortSaleRestriction::from_code(PROFILE_SSR.get32(flags))
        }

        /// Whether short sales of the security are restricted.
        #[doc(alias = "isShortSaleRestricted")]
        fn is_short_sale_restricted(flags) -> bool {
            ShortSaleRestriction::from_code(PROFILE_SSR.get32(flags)) == ShortSaleRestriction::Active
        }

        /// The trading status of the security.
        #[doc(alias = "getTradingStatus")]
        fn trading_status(flags) -> TradingStatus {
            TradingStatus::from_code(PROFILE_STATUS.get32(flags))
        }

        /// Whether trading of the security is halted.
        #[doc(alias = "isTradingHalted")]
        fn is_trading_halted(flags) -> bool {
            TradingStatus::from_code(PROFILE_STATUS.get32(flags)) == TradingStatus::Halted
        }
    }
}

impl Profile {
    /// Changes the short sale restriction of the security.
    #[doc(alias = "setShortSaleRestriction")]
    pub fn set_short_sale_restriction(&mut self, restriction: ShortSaleRestriction) {
        self.flags = PROFILE_SSR.set32(self.flags, restriction.get_code());
    }

    /// Changes the trading status of the security.
    #[doc(alias = "setTradingStatus")]
    pub fn set_trading_status(&mut self, status: TradingStatus) {
        self.flags = PROFILE_STATUS.set32(self.flags, status.get_code());
    }
}

/// The text of Java's `Profile.toString`, e.g. `Profile{AAPL, eventTime=0, description='Apple Inc.', ...}`; the halt
/// times print without milliseconds, the shares and the free float as Java's `WideDecimal`.
impl std::fmt::Display for Profile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Profile{{{}, eventTime={}, description={}, SSR={}, status={}, statusReason={}, haltStartTime={}, \
             haltEndTime={}, highLimitPrice={}, lowLimitPrice={}, high52WeekPrice={}, low52WeekPrice={}, beta={}, \
             earningsPerShare={}, dividendFrequency={}, exDividendAmount={}, exDividendDay={}, shares={}, \
             freeFloat={}}}",
            self.symbol,
            Time(self.event_time),
            Quoted(self.description.as_deref()),
            self.short_sale_restriction(),
            self.trading_status(),
            Quoted(self.status_reason.as_deref()),
            TimeSeconds(self.halt_start_time),
            TimeSeconds(self.halt_end_time),
            JavaDouble(self.high_limit_price),
            JavaDouble(self.low_limit_price),
            JavaDouble(self.high_52_week_price),
            JavaDouble(self.low_52_week_price),
            JavaDouble(self.beta),
            JavaDouble(self.earnings_per_share),
            JavaDouble(self.dividend_frequency),
            JavaDouble(self.ex_dividend_amount),
            year_month_day(self.ex_dividend_day_id),
            WideDecimal(self.shares),
            WideDecimal(self.free_float),
        )
    }
}
