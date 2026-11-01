//! Series events.

use dxfeed_graal_sys as sys;

use super::format::{JavaDouble, Time, year_month_day};

time_sequence_values! {
    time_sequence
    event_type! {
        head {
            /// The properties of an option series: its volatility, volumes and the forward price (`Series`).
            Series, SeriesRef, sys::dxfg_series_t, sys::DXFG_EVENT_SERIES, [market_event]
        }
        fields {
            /// The flags of the event in a transaction or a snapshot.
            event_flags: event_flags [event_flags] = super::EventFlags::empty(),
            /// The index of the series among the series of its symbol.
            index: i64 [index] = 0,
            /// The time and the sequence of the event, packed as [`time`](Self::time) and
            /// [`sequence`](Self::sequence) read them.
            time_sequence: i64 [time_sequence] = 0,
            /// The day of the expiration of the series, in days since 1970-01-01.
            expiration: i32 [expiration] = 0,
            /// The implied volatility of the series; NaN if unknown.
            volatility: f64 [volatility] = f64::NAN,
            /// The call options of the series traded today; NaN if unknown.
            call_volume: f64 [call_volume] = f64::NAN,
            /// The put options of the series traded today; NaN if unknown.
            put_volume: f64 [put_volume] = f64::NAN,
            /// The ratio of the put volume to the call volume; NaN if unknown.
            put_call_ratio: f64 [put_call_ratio] = f64::NAN,
            /// The implied forward price of the underlying; NaN if unknown.
            forward_price: f64 [forward_price] = f64::NAN,
            /// The implied simple dividend return of the underlying; NaN if unknown.
            dividend: f64 [dividend] = f64::NAN,
            /// The implied simple interest return of the underlying; NaN if unknown.
            interest: f64 [interest] = f64::NAN,
        }
        derived {
            /// The options traded today, puts and calls, as Java's `getOptionVolume` gives it: one of the volumes if
            /// the other is NaN, else their sum.
            #[doc(alias = "getOptionVolume")]
            fn option_volume(put_volume, call_volume) -> f64 {
                if put_volume.is_nan() {
                    call_volume
                } else if call_volume.is_nan() {
                    put_volume
                } else {
                    put_volume + call_volume
                }
            }

            /// The source of the event: always the default source `DEFAULT`, as Java's `getSource` gives it.
            #[doc(alias = "getSource")]
            fn source() -> super::IndexedEventSource {
                super::IndexedEventSource::DEFAULT
            }
        }
    }
}

/// The text of Java's `Series.toString`, e.g. `Series{AAPL, eventTime=0, eventFlags=0x0, index=0x11, ...}`.
impl std::fmt::Display for Series {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Series{{{}, eventTime={}, eventFlags=0x{:x}, index=0x{:x}, time={}, sequence={}, expiration={}, \
             volatility={}, callVolume={}, putVolume={}, putCallRatio={}, forwardPrice={}, dividend={}, interest={}}}",
            self.symbol,
            Time(self.event_time),
            self.event_flags,
            self.index,
            Time(self.time()),
            self.sequence(),
            year_month_day(self.expiration),
            JavaDouble(self.volatility),
            JavaDouble(self.call_volume),
            JavaDouble(self.put_volume),
            JavaDouble(self.put_call_ratio),
            JavaDouble(self.forward_price),
            JavaDouble(self.dividend),
            JavaDouble(self.interest),
        )
    }
}
