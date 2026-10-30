//! The tables that generate the event types.
//!
//! Each event type is one [`event_type!`] table: the names of the owned type and its view, the SDK struct, the event
//! class and the path to the symbol and the event time, then one row per field and one entry per value computed from
//! fields. A field-group macro, such as [`trade_base_fields!`], adds the rows several types share and calls the table
//! with them. Each table also generates the offline tests of its type: they need the SDK library but no endpoint.
//!
//! A row is `name: kind [path.in.the.sdk.struct] = default`. The kinds are the number types `f64`, `i64` and `i32`,
//! `char` for a Java `char` code kept in an `i16`, `event_flags` for the [`EventFlags`](super::EventFlags) kept in an
//! `i32`, `string` for a nullable UTF-8 string and `attachment` for the nullable UTF-8 string the SDK keeps in a
//! `void *`.

/// Generates an event type from its table: the owned type with `new`, its pub fields and computed values, the view
/// with one accessor per field and value, `to_owned`, `Debug` and `Display` of the view, the conversion for
/// publishing and the offline tests. The `Display` of the owned type is written by hand next to the table.
macro_rules! event_type {
    (
        head {
            $(#[$doc:meta])*
            $name:ident, $view:ident, $sys:ty, $class:path, [$($h:ident).*]
        }
        fields {
            $( $(#[$fdoc:meta])* $field:ident: $kind:ident [$($p:ident).+] = $default:expr, )*
        }
        derived {
            $( $(#[$ddoc:meta])* fn $dname:ident($($arg:ident),*) -> $dty:ty $body:block )*
        }
    ) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq)]
        pub struct $name {
            /// The symbol.
            pub symbol: String,
            /// The time the event was created, in milliseconds since the Unix epoch; `0` if unknown.
            pub event_time: i64,
            $( $(#[$fdoc])* pub $field: event_field!(@owned $kind), )*
        }

        impl $name {
            #[doc = concat!(
                "A `", stringify!($name), "` for `symbol` with the default values of Java's `new ",
                stringify!($name), "(symbol)`."
            )]
            pub fn new(symbol: &str) -> Self {
                $name {
                    symbol: symbol.to_owned(),
                    event_time: 0,
                    $( $field: $default, )*
                }
            }

            $(
                $(#[$ddoc])*
                pub fn $dname(&self) -> $dty {
                    $( let $arg = self.$arg; )*
                    $body
                }
            )*
        }

        #[doc = concat!(
            "A [`", stringify!($name), "`] received by a listener, borrowed from the SDK: valid during the listener ",
            "call only."
        )]
        #[repr(transparent)]
        pub struct $view($sys);

        impl $view {
            /// The symbol; replacement characters stand for bytes that are not UTF-8.
            pub fn symbol(&self) -> ::std::borrow::Cow<'_, str> {
                // SAFETY: the SDK keeps the symbol alive while the event lives.
                unsafe { $crate::isolate::string(self.0 $(.$h)* .event_symbol) }
            }

            /// The time the event was created, in milliseconds since the Unix epoch; `0` if unknown.
            pub fn event_time(&self) -> i64 {
                self.0 $(.$h)* .event_time
            }

            $(
                $(#[$fdoc])*
                pub fn $field(&self) -> event_field!(@view $kind) {
                    event_field!(@get $kind, self.0.$($p).+)
                }
            )*

            $(
                $(#[$ddoc])*
                pub fn $dname(&self) -> $dty {
                    $( let $arg = self.$arg(); )*
                    $body
                }
            )*

            /// Copies the event.
            pub fn to_owned(&self) -> $name {
                $name {
                    symbol: self.symbol().into_owned(),
                    event_time: self.event_time(),
                    $( $field: event_field!(@to_owned $kind, self.$field()), )*
                }
            }
        }

        impl ::std::fmt::Debug for $view {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                ::std::fmt::Debug::fmt(&self.to_owned(), f)
            }
        }

        #[doc = concat!("The text of Java's `", stringify!($name), ".toString`, as the owned event prints it.")]
        impl ::std::fmt::Display for $view {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                ::std::fmt::Display::fmt(&self.to_owned(), f)
            }
        }

        impl $crate::event::Table for $name {
            type Sys = $sys;
            const CLASS: ::dxfeed_graal_sys::dxfg_event_clazz_t = $class;

            fn to_sys(&self, strings: &mut Vec<::std::ffi::CString>) -> $crate::error::Result<$sys> {
                // SAFETY: the SDK struct is plain data (integers, floats and pointers) for which all zeros are valid.
                let mut raw: $sys = unsafe { ::std::mem::zeroed() };
                raw $(.$h)* .event_type.clazz = $class;
                raw $(.$h)* .event_symbol = $crate::event::arena_string(strings, Some(&self.symbol), "event symbol")?;
                raw $(.$h)* .event_time = self.event_time;
                $(
                    raw.$($p).+ = event_field!(
                        @to_sys $kind, &self.$field, strings, concat!(stringify!($name), " ", stringify!($field))
                    );
                )*
                Ok(raw)
            }
        }

        #[cfg(test)]
        mod table_tests {
            use super::*;
            use $crate::event::Table;

            fn view(raw: &$sys) -> &$view {
                // SAFETY: the view is a transparent wrapper of the SDK struct.
                unsafe { &*(raw as *const $sys).cast::<$view>() }
            }

            fn header(raw: &mut $sys, strings: &mut Vec<::std::ffi::CString>, event_time: i64) {
                raw $(.$h)* .event_type.clazz = $class;
                raw $(.$h)* .event_symbol = $crate::event::arena_string(strings, Some("SYM"), "").unwrap();
                raw $(.$h)* .event_time = event_time;
            }

            #[test]
            fn view_has_the_layout_of_the_sdk_struct() {
                assert_eq!(::std::mem::size_of::<$view>(), ::std::mem::size_of::<$sys>());
                assert_eq!(::std::mem::align_of::<$view>(), ::std::mem::align_of::<$sys>());
                assert_eq!(::std::mem::offset_of!($sys, $($h.)* event_type), 0);
            }

            #[test]
            fn fields_convert_both_ways() {
                let mut strings = Vec::new();
                // SAFETY: as in `to_sys`.
                let mut raw: $sys = unsafe { ::std::mem::zeroed() };
                header(&mut raw, &mut strings, 123_456_789);
                let mut n: u32 = 0;
                $( n += 1; raw.$($p).+ = event_field!(@sample_sys $kind, n, &mut strings); )*

                let owned = view(&raw).to_owned();
                assert_eq!(owned.symbol, "SYM");
                assert_eq!(owned.event_time, 123_456_789);
                let mut n: u32 = 0;
                $( n += 1; assert_eq!(owned.$field, event_field!(@sample $kind, n), stringify!($field)); )*

                let mut back_strings = Vec::new();
                let back = owned.to_sys(&mut back_strings).unwrap();
                assert_eq!(back $(.$h)* .event_type.clazz, $class);
                assert_eq!(view(&back).to_owned(), owned);
            }

            #[test]
            fn zeroed_event_reads_as_zero_and_null() {
                // SAFETY: as in `to_sys`.
                let raw: $sys = unsafe { ::std::mem::zeroed() };
                let owned = view(&raw).to_owned();
                assert_eq!(owned.symbol, "");
                $( event_field!(@assert_zero $kind, owned.$field); )*
                let _ = (view(&raw).to_string(), owned.to_string(), format!("{:?}", view(&raw)));
            }

            #[test]
            fn extreme_values_neither_panic_nor_fail() {
                for pattern in 0..6 {
                    let mut strings = Vec::new();
                    // SAFETY: as in `to_sys`.
                    let mut raw: $sys = unsafe { ::std::mem::zeroed() };
                    header(&mut raw, &mut strings, event_field!(@extreme i64, pattern, &mut strings));
                    $( raw.$($p).+ = event_field!(@extreme $kind, pattern, &mut strings); )*
                    let view = view(&raw);
                    let owned = view.to_owned();
                    let _ = (view.to_string(), owned.to_string(), format!("{view:?}"));
                    $( let _ = (view.$dname(), owned.$dname()); )*
                    owned.to_sys(&mut Vec::new()).unwrap();
                }
            }
        }
    };
}

/// The parts of [`event_type!`] that depend on the kind of a field.
macro_rules! event_field {
    (@owned string) => { Option<String> };
    (@owned attachment) => { Option<String> };
    (@owned event_flags) => { $crate::event::EventFlags };
    (@owned $kind:ident) => { $kind };

    (@view string) => { Option<::std::borrow::Cow<'_, str>> };
    (@view attachment) => { Option<::std::borrow::Cow<'_, str>> };
    (@view event_flags) => { $crate::event::EventFlags };
    (@view $kind:ident) => { $kind };

    (@get char, $v:expr) => { $crate::event::exchange_code($v) };
    (@get event_flags, $v:expr) => { $crate::event::EventFlags::from_bits($v) };
    // SAFETY: the SDK keeps the strings of an event alive while the event lives.
    (@get string, $v:expr) => { unsafe { $crate::event::opt_string($v) } };
    // SAFETY: as above; the attachment is a UTF-8 string or NULL.
    (@get attachment, $v:expr) => { unsafe { $crate::event::opt_string($v.cast_const().cast()) } };
    (@get $kind:ident, $v:expr) => { $v };

    (@to_owned string, $v:expr) => { $v.map(::std::borrow::Cow::into_owned) };
    (@to_owned attachment, $v:expr) => { $v.map(::std::borrow::Cow::into_owned) };
    (@to_owned $kind:ident, $v:expr) => { $v };

    (@to_sys char, $v:expr, $strings:expr, $what:expr) => { $crate::event::sys_exchange_code(*$v)? };
    (@to_sys event_flags, $v:expr, $strings:expr, $what:expr) => { $v.bits() };
    (@to_sys string, $v:expr, $strings:expr, $what:expr) => {
        $crate::event::arena_string($strings, $v.as_deref(), $what)?
    };
    (@to_sys attachment, $v:expr, $strings:expr, $what:expr) => {
        $crate::event::arena_string($strings, $v.as_deref(), $what)?.cast_mut().cast()
    };
    (@to_sys $kind:ident, $v:expr, $strings:expr, $what:expr) => { *$v };

    // A value distinct for each field number `n`, in the SDK struct and as the owned event keeps it.
    (@sample_sys char, $n:expr, $strings:expr) => { (0x40 + $n) as i16 };
    (@sample_sys event_flags, $n:expr, $strings:expr) => { event_field!(@sample i32, $n) };
    (@sample_sys string, $n:expr, $strings:expr) => {
        $crate::event::arena_string($strings, Some(format!("s{}", $n).as_str()), "").unwrap()
    };
    (@sample_sys attachment, $n:expr, $strings:expr) => {
        event_field!(@sample_sys string, $n, $strings).cast_mut().cast()
    };
    (@sample_sys $kind:ident, $n:expr, $strings:expr) => { event_field!(@sample $kind, $n) };
    (@sample f64, $n:expr) => { f64::from($n) + 0.25 };
    (@sample i64, $n:expr) => { i64::from($n) * 1_000_003 };
    (@sample i32, $n:expr) => { $n as i32 * 7 + 1 };
    (@sample char, $n:expr) => { char::from(0x40 + $n as u8) };
    (@sample event_flags, $n:expr) => { $crate::event::EventFlags::from_bits(event_field!(@sample i32, $n)) };
    (@sample string, $n:expr) => { Some(format!("s{}", $n)) };
    (@sample attachment, $n:expr) => { Some(format!("s{}", $n)) };

    (@assert_zero f64, $v:expr) => { assert_eq!($v, 0.0) };
    (@assert_zero char, $v:expr) => { assert_eq!($v, '\0') };
    (@assert_zero event_flags, $v:expr) => { assert_eq!($v, $crate::event::EventFlags::empty()) };
    (@assert_zero string, $v:expr) => { assert_eq!($v, None) };
    (@assert_zero attachment, $v:expr) => { assert_eq!($v, None) };
    (@assert_zero $kind:ident, $v:expr) => { assert_eq!($v, 0) };

    // Pattern `p` (0 to 5) of extreme values: the limits, all bits set, NaN, infinities, invalid strings.
    (@extreme f64, $p:expr, $strings:expr) => {
        [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -0.0, 5e-324, f64::MAX][$p]
    };
    (@extreme i64, $p:expr, $strings:expr) => { [i64::MIN, i64::MAX, -1, 0, 1, i64::MIN + 1][$p] };
    (@extreme i32, $p:expr, $strings:expr) => { [i32::MIN, i32::MAX, -1, 0, 1, i32::MIN + 1][$p] };
    (@extreme event_flags, $p:expr, $strings:expr) => { event_field!(@extreme i32, $p, $strings) };
    (@extreme char, $p:expr, $strings:expr) => { [i16::MIN, i16::MAX, -1, 0, 0x7F, 0xD800_u16 as i16][$p] };
    (@extreme string, $p:expr, $strings:expr) => { $crate::event::extreme_string($strings, $p) };
    (@extreme attachment, $p:expr, $strings:expr) => {
        $crate::event::extreme_string($strings, $p).cast_mut().cast()
    };
}

/// The time and the sequence of an event packed in its field `ts` (seconds in bits 32 to 63, milliseconds in bits 22
/// to 31, the sequence in bits 0 to 21), as Java's `getTime`, `getSequence`, `setTime` and `setSequence` read and write
/// them, for [`event_type!`].
macro_rules! time_sequence_values {
    (
        $ts:ident
        $cb:ident! {
            head { $(#[$doc:meta])* $name:ident, $($head:tt)* }
            fields { $($fields:tt)* }
            derived { $($derived:tt)* }
        }
    ) => {
        impl $name {
            /// Changes the time of the event and keeps the sequence, as Java's `setTime` does: the seconds are clamped
            /// to 32 bits.
            #[doc(alias = "setTime")]
            pub fn set_time(&mut self, time: i64) {
                self.$ts = $crate::event::flags::with_time(self.$ts, time);
            }

            /// Changes the sequence of the event.
            ///
            /// # Errors
            ///
            /// [`Error::InvalidArgument`]($crate::Error::InvalidArgument) if the sequence is negative or above
            /// `(1 << 22) - 1`.
            #[doc(alias = "setSequence")]
            pub fn set_sequence(&mut self, sequence: i32) -> $crate::Result<()> {
                self.$ts = $crate::event::flags::with_sequence(self.$ts, sequence)?;
                Ok(())
            }
        }

        $cb! {
            head { $(#[$doc])* $name, $($head)* }
            fields { $($fields)* }
            derived {
                /// The time of the event, in milliseconds since the Unix epoch.
                #[doc(alias = "getTime")]
                fn time($ts) -> i64 {
                    $crate::event::flags::time_of($ts)
                }

                /// The sequence number of the event, to tell apart events with the same time.
                #[doc(alias = "getSequence")]
                fn sequence($ts) -> i32 {
                    $crate::event::flags::sequence_of($ts)
                }

                $($derived)*
            }
        }
    };
}

/// The fields of Java's time series events that keep the time and the sequence in the index (Greeks, Underlying,
/// TheoPrice, TimeAndSale, Candle) at the path `[p]`, with the values of [`time_sequence_values!`], for
/// [`event_type!`].
macro_rules! time_series_fields {
    (
        [$($p:ident).*]
        $cb:ident! {
            head { $($head:tt)* }
            fields { $($fields:tt)* }
            derived { $($derived:tt)* }
        }
    ) => {
        time_sequence_values! {
            index
            $cb! {
                head { $($head)* }
                fields {
                    /// The flags of the event in a transaction or a snapshot.
                    event_flags: event_flags [$($p.)* event_flags] = $crate::event::EventFlags::empty(),
                    /// The index of the event: its time and sequence, packed as [`time`](Self::time) and
                    /// [`sequence`](Self::sequence) read them.
                    index: i64 [$($p.)* index] = 0,
                    $($fields)*
                }
                derived {
                    /// The id of the source of the event: always `0`, the default source `DEFAULT`, as Java's
                    /// `getSource` gives it.
                    #[doc(alias = "getSource")]
                    fn source() -> i32 {
                        0
                    }

                    $($derived)*
                }
            }
        }
    };
}

/// The fields, values and setters of Java's `TradeBase` at the path `[p]`, for [`event_type!`].
macro_rules! trade_base_fields {
    (
        [$($p:ident).*]
        $cb:ident! {
            head { $(#[$doc:meta])* $name:ident, $($head:tt)* }
            fields { $($fields:tt)* }
            derived { $($derived:tt)* }
        }
    ) => {
        impl $name {
            /// Writes the text of Java's `TradeBase.baseFieldsToString` after `{`: the symbol to `ETH=...`.
            fn fmt_trade_base(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                use $crate::event::format::{JavaChar, JavaDouble, Time, year_month_day};
                write!(
                    f,
                    "{}, eventTime={}, time={}, timeNanoPart={}, sequence={}, exchange={}, tradeId={}, price={}, \
                     change={}, size={}, day={}, dayVolume={}, dayTurnover={}, direction={}, ETH={}",
                    self.symbol,
                    Time(self.event_time),
                    Time(self.time()),
                    self.time_nano_part,
                    self.sequence(),
                    JavaChar(self.exchange_code),
                    self.trade_id,
                    JavaDouble(self.price),
                    JavaDouble(self.change),
                    JavaDouble(self.size),
                    year_month_day(self.day_id),
                    JavaDouble(self.day_volume),
                    JavaDouble(self.day_turnover),
                    self.tick_direction(),
                    self.is_extended_trading_hours(),
                )
            }

            /// Changes the time of the trade and its nanoseconds part, as Java's `setTimeNanos` does.
            #[doc(alias = "setTimeNanos")]
            pub fn set_time_nanos(&mut self, time_nanos: i64) {
                let (time, nano_part) = $crate::event::flags::split_nanos(time_nanos);
                self.set_time(time);
                self.time_nano_part = nano_part;
            }

            /// Changes the direction of the price movement from the previous trade.
            #[doc(alias = "setTickDirection")]
            pub fn set_tick_direction(&mut self, direction: $crate::event::Direction) {
                self.flags = $crate::event::flags::TRADE_DIRECTION.set32(self.flags, direction.code());
            }

            /// Changes whether the trade happened in extended trading hours.
            #[doc(alias = "setExtendedTradingHours")]
            pub fn set_extended_trading_hours(&mut self, extended_trading_hours: bool) {
                self.flags = $crate::event::flags::TRADE_ETH.set32(self.flags, extended_trading_hours.into());
            }
        }

        time_sequence_values! {
            time_sequence
            $cb! {
            head { $(#[$doc])* $name, $($head)* }
            fields {
                /// The time and the sequence of the trade, packed as the SDK keeps them: seconds in bits 32 to 63,
                /// milliseconds in bits 22 to 31 and the sequence in bits 0 to 21.
                time_sequence: i64 [$($p.)* time_sequence] = 0,
                /// The microseconds and nanoseconds part of the trade time.
                time_nano_part: i32 [$($p.)* time_nano_part] = 0,
                /// The exchange code of the trade; it must fit a Java `char`.
                exchange_code: char [$($p.)* exchange_code] = '\0',
                /// The price; NaN if unknown.
                price: f64 [$($p.)* price] = f64::NAN,
                /// The change of the price from the close of the previous day; NaN if unknown.
                change: f64 [$($p.)* change] = f64::NAN,
                /// The size; NaN if unknown.
                size: f64 [$($p.)* size] = f64::NAN,
                /// The identifier of the trading day, in days since 1970-01-01.
                day_id: i32 [$($p.)* day_id] = 0,
                /// The volume traded during the day; NaN if unknown.
                day_volume: f64 [$($p.)* day_volume] = f64::NAN,
                /// The turnover during the day; NaN if unknown.
                day_turnover: f64 [$($p.)* day_turnover] = f64::NAN,
                /// The flags of the trade, packed as the SDK keeps them: the tick direction in bits 1 to 3 and the
                /// extended trading hours in bit 0.
                flags: i32 [$($p.)* flags] = 0,
                /// The identifier of the trade.
                trade_id: i64 [$($p.)* trade_id] = 0,
                $($fields)*
            }
            derived {
                /// The time of the trade, in nanoseconds since the Unix epoch.
                #[doc(alias = "getTimeNanos")]
                fn time_nanos(time_sequence, time_nano_part) -> i64 {
                    $crate::event::flags::nanos($crate::event::flags::time_of(time_sequence), time_nano_part)
                }

                /// The direction of the price movement from the previous trade.
                #[doc(alias = "getTickDirection")]
                fn tick_direction(flags) -> $crate::event::Direction {
                    $crate::event::Direction::from_code($crate::event::flags::TRADE_DIRECTION.get32(flags))
                }

                /// Whether the trade happened in extended trading hours.
                #[doc(alias = "isExtendedTradingHours")]
                fn is_extended_trading_hours(flags) -> bool {
                    $crate::event::flags::TRADE_ETH.is_set32(flags)
                }

                $($derived)*
            }
            }
        }
    };
}

/// The fields, values and setters that Java's `TimeAndSale` and `OptionSale` share, at the path `[p]`, for
/// [`event_type!`]. With `index`, the index keeps the time and the sequence, as in `TimeAndSale`; with
/// `time_sequence`, a field of its own keeps them next to the index, as in `OptionSale`.
macro_rules! sale_fields {
    (
        [$($p:ident).*] index
        $cb:ident! {
            head { $(#[$doc:meta])* $name:ident, $($head:tt)* }
            fields { $($fields:tt)* }
            derived { $($derived:tt)* }
        }
    ) => {
        sale_fields! {
            @sale [$($p).*] index $name
            time_series_fields! { [$($p).*] }
            {}
            $cb! { head { $(#[$doc])* $name, $($head)* } fields { $($fields)* } derived { $($derived)* } }
        }
    };
    (
        [$($p:ident).*] time_sequence
        $cb:ident! {
            head { $(#[$doc:meta])* $name:ident, $($head:tt)* }
            fields { $($fields:tt)* }
            derived { $($derived:tt)* }
        }
    ) => {
        sale_fields! {
            @sale [$($p).*] time_sequence $name
            time_sequence_values! { time_sequence }
            {
                /// The flags of the event in a transaction or a snapshot.
                event_flags: event_flags [$($p.)* event_flags] = $crate::event::EventFlags::empty(),
                /// The index of the event among the events of its symbol, to correct or cancel it later.
                index: i64 [$($p.)* index] = 0,
                /// The time and the sequence of the event, packed as [`time`](Self::time) and
                /// [`sequence`](Self::sequence) read them.
                time_sequence: i64 [$($p.)* time_sequence] = 0,
            }
            $cb! {
                head { $(#[$doc])* $name, $($head)* }
                fields { $($fields)* }
                derived {
                    /// The id of the source of the event: always `0`, the default source `DEFAULT`, as Java's
                    /// `getSource` gives it.
                    #[doc(alias = "getSource")]
                    fn source() -> i32 {
                        0
                    }

                    $($derived)*
                }
            }
        }
    };
    (
        @sale [$($p:ident).*] $ts:ident $name:ident
        $group:ident! { $($group_args:tt)* }
        { $($first:tt)* }
        $cb:ident! {
            head { $($head:tt)* }
            fields { $($fields:tt)* }
            derived { $($derived:tt)* }
        }
    ) => {
        impl $name {
            /// Writes the part of Java's `toString` from `price=` to `type=...`.
            fn fmt_sale(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                use $crate::event::format::{JavaChar, JavaDouble, Quoted};
                write!(
                    f,
                    "price={}, size={}, bid={}, ask={}, ESC={}, TTE={}, side={}, spread={}, ETH={}, validTick={}, \
                     type={}",
                    JavaDouble(self.price),
                    JavaDouble(self.size),
                    JavaDouble(self.bid_price),
                    JavaDouble(self.ask_price),
                    Quoted(self.exchange_sale_conditions.as_deref()),
                    JavaChar(self.trade_through_exempt()),
                    self.aggressor_side(),
                    self.is_spread_leg(),
                    self.is_extended_trading_hours(),
                    self.is_valid_tick(),
                    self.time_and_sale_type(),
                )
            }

            /// Changes the time of the event and its nanoseconds part, as Java's `setTimeNanos` does.
            #[doc(alias = "setTimeNanos")]
            pub fn set_time_nanos(&mut self, time_nanos: i64) {
                let (time, nano_part) = $crate::event::flags::split_nanos(time_nanos);
                self.set_time(time);
                self.time_nano_part = nano_part;
            }

            /// Changes the trade-through-exempt code.
            ///
            /// # Errors
            ///
            /// [`Error::InvalidArgument`]($crate::Error::InvalidArgument) if the code is above `'\u{ff}'`.
            #[doc(alias = "setTradeThroughExempt")]
            pub fn set_trade_through_exempt(&mut self, code: char) -> $crate::Result<()> {
                let bits = u8::try_from(code).map_err(|_| {
                    $crate::Error::InvalidArgument(format!("trade-through-exempt code {code:?} does not fit 8 bits"))
                })?;
                self.flags = $crate::event::flags::SALE_TTE.set32(self.flags, bits.into());
                Ok(())
            }

            /// Changes the side of the aggressor of the trade.
            #[doc(alias = "setAggressorSide")]
            pub fn set_aggressor_side(&mut self, side: $crate::event::Side) {
                self.flags = $crate::event::flags::SALE_SIDE.set32(self.flags, side.code());
            }

            /// Changes whether the event is a leg of a spread trade.
            #[doc(alias = "setSpreadLeg")]
            pub fn set_spread_leg(&mut self, spread_leg: bool) {
                self.flags = $crate::event::flags::SALE_SPREAD_LEG.set32(self.flags, spread_leg.into());
            }

            /// Changes whether the event happened in extended trading hours.
            #[doc(alias = "setExtendedTradingHours")]
            pub fn set_extended_trading_hours(&mut self, extended_trading_hours: bool) {
                self.flags = $crate::event::flags::SALE_ETH.set32(self.flags, extended_trading_hours.into());
            }

            /// Changes whether the event is a valid tick.
            #[doc(alias = "setValidTick")]
            pub fn set_valid_tick(&mut self, valid_tick: bool) {
                self.flags = $crate::event::flags::SALE_VALID_TICK.set32(self.flags, valid_tick.into());
            }

            /// Changes the type of the event: new, correction or cancellation.
            #[doc(alias = "setType")]
            pub fn set_time_and_sale_type(&mut self, sale_type: $crate::event::TimeAndSaleType) {
                self.flags = $crate::event::flags::SALE_TYPE.set32(self.flags, sale_type.code());
            }
        }

        $group! {
            $($group_args)*
            $cb! {
                head { $($head)* }
                fields {
                    $($first)*
                    /// The microseconds and nanoseconds part of the time of the event.
                    time_nano_part: i32 [$($p.)* time_nano_part] = 0,
                    /// The exchange code of the event; it must fit a Java `char`.
                    exchange_code: char [$($p.)* exchange_code] = '\0',
                    /// The price; NaN if unknown.
                    price: f64 [$($p.)* price] = f64::NAN,
                    /// The size; NaN if unknown.
                    size: f64 [$($p.)* size] = f64::NAN,
                    /// The bid price on the market at the time of the event; NaN if unknown.
                    bid_price: f64 [$($p.)* bid_price] = f64::NAN,
                    /// The ask price on the market at the time of the event; NaN if unknown.
                    ask_price: f64 [$($p.)* ask_price] = f64::NAN,
                    /// The sale conditions of the data feed, in its own format; `None` if none.
                    exchange_sale_conditions: string [$($p.)* exchange_sale_conditions] = None,
                    /// The flags of the event, packed as the SDK keeps them: the trade-through-exempt code in bits 8
                    /// to 15, the aggressor side in bits 5 and 6, the spread leg, extended trading hours and valid
                    /// tick in bits 4, 3 and 2, the type in bits 0 and 1.
                    flags: i32 [$($p.)* flags] = 0,
                    $($fields)*
                }
                derived {
                    /// The time of the event, in nanoseconds since the Unix epoch.
                    #[doc(alias = "getTimeNanos")]
                    fn time_nanos($ts, time_nano_part) -> i64 {
                        $crate::event::flags::nanos($crate::event::flags::time_of($ts), time_nano_part)
                    }

                    /// The trade-through-exempt code; `'\0'` if none.
                    #[doc(alias = "getTradeThroughExempt")]
                    fn trade_through_exempt(flags) -> char {
                        char::from($crate::event::flags::SALE_TTE.get32(flags) as u8)
                    }

                    /// The side of the aggressor of the trade.
                    #[doc(alias = "getAggressorSide")]
                    fn aggressor_side(flags) -> $crate::event::Side {
                        $crate::event::Side::from_code($crate::event::flags::SALE_SIDE.get32(flags))
                    }

                    /// Whether the event is a leg of a spread trade.
                    #[doc(alias = "isSpreadLeg")]
                    fn is_spread_leg(flags) -> bool {
                        $crate::event::flags::SALE_SPREAD_LEG.is_set32(flags)
                    }

                    /// Whether the event happened in extended trading hours.
                    #[doc(alias = "isExtendedTradingHours")]
                    fn is_extended_trading_hours(flags) -> bool {
                        $crate::event::flags::SALE_ETH.is_set32(flags)
                    }

                    /// Whether the event is a valid tick: a trade that may update the last price and the day
                    /// statistics.
                    #[doc(alias = "isValidTick")]
                    fn is_valid_tick(flags) -> bool {
                        $crate::event::flags::SALE_VALID_TICK.is_set32(flags)
                    }

                    /// The type of the event: new, correction or cancellation.
                    #[doc(alias = "getType")]
                    fn time_and_sale_type(flags) -> $crate::event::TimeAndSaleType {
                        $crate::event::TimeAndSaleType::from_code($crate::event::flags::SALE_TYPE.get32(flags))
                    }

                    /// Whether the event is new: neither a correction nor a cancellation.
                    #[doc(alias = "isNew")]
                    fn is_new(flags) -> bool {
                        $crate::event::flags::SALE_TYPE.get32(flags) == $crate::event::TimeAndSaleType::New.code()
                    }

                    /// Whether the event corrects an earlier event.
                    #[doc(alias = "isCorrection")]
                    fn is_correction(flags) -> bool {
                        $crate::event::flags::SALE_TYPE.get32(flags)
                            == $crate::event::TimeAndSaleType::Correction.code()
                    }

                    /// Whether the event cancels an earlier event.
                    #[doc(alias = "isCancel")]
                    fn is_cancel(flags) -> bool {
                        $crate::event::flags::SALE_TYPE.get32(flags) == $crate::event::TimeAndSaleType::Cancel.code()
                    }

                    $($derived)*
                }
            }
        }
    };
}

/// The fields of Java's `TimeAndSale` at the path `[p]`, with the values and setters of [`sale_fields!`], for
/// [`event_type!`].
macro_rules! time_and_sale_fields {
    (
        [$($p:ident).*]
        $cb:ident! {
            head { $(#[$doc:meta])* $name:ident, $($head:tt)* }
            fields { $($fields:tt)* }
            derived { $($derived:tt)* }
        }
    ) => {
        impl $name {
            /// Writes the text of Java's `TimeAndSale.toString` after `{` and before `}`.
            fn fmt_time_and_sale(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                use $crate::event::format::{JavaChar, Quoted, Time};
                write!(
                    f,
                    "{}, eventTime={}, eventFlags=0x{:x}, time={}, timeNanoPart={}, sequence={}, exchange={}, \
                     tradeId={}, ",
                    self.symbol,
                    Time(self.event_time),
                    self.event_flags,
                    Time(self.time()),
                    self.time_nano_part,
                    self.sequence(),
                    JavaChar(self.exchange_code),
                    self.trade_id,
                )?;
                self.fmt_sale(f)?;
                // Java prints the buyer and the seller only when they are set.
                if let Some(buyer) = self.buyer.as_deref() {
                    write!(f, ", buyer={}", Quoted(Some(buyer)))?;
                }
                if let Some(seller) = self.seller.as_deref() {
                    write!(f, ", seller={}", Quoted(Some(seller)))?;
                }
                Ok(())
            }
        }

        sale_fields! {
            [$($p).*] index
            $cb! {
                head { $(#[$doc])* $name, $($head)* }
                fields {
                    /// The identifier of the trade for the events of a trade (execution, correction, cancellation);
                    /// `0` if none.
                    trade_id: i64 [$($p.)* trade_id] = 0,
                    /// The buyer; `None` if unknown.
                    buyer: string [$($p.)* buyer] = None,
                    /// The seller; `None` if unknown.
                    seller: string [$($p.)* seller] = None,
                    $($fields)*
                }
                derived { $($derived)* }
            }
        }
    };
}

/// The fields of Java's `Candle` at the path `[p]`, with the values of [`time_series_fields!`], for [`event_type!`].
macro_rules! candle_fields {
    (
        [$($p:ident).*]
        $cb:ident! {
            head { $(#[$doc:meta])* $name:ident, $($head:tt)* }
            fields { $($fields:tt)* }
            derived { $($derived:tt)* }
        }
    ) => {
        impl $name {
            /// Writes the text of Java's `Candle.toString` after `{` and before `}`.
            fn fmt_candle(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                use $crate::event::format::{JavaDouble, Time};
                write!(
                    f,
                    "{}, eventTime={}, eventFlags=0x{:x}, time={}, sequence={}, count={}, open={}, high={}, low={}, \
                     close={}, volume={}, vwap={}, bidVolume={}, askVolume={}, impVolatility={}, openInterest={}",
                    self.symbol,
                    Time(self.event_time),
                    self.event_flags,
                    Time(self.time()),
                    self.sequence(),
                    self.count,
                    JavaDouble(self.open),
                    JavaDouble(self.high),
                    JavaDouble(self.low),
                    JavaDouble(self.close),
                    JavaDouble(self.volume),
                    JavaDouble(self.vwap),
                    JavaDouble(self.bid_volume),
                    JavaDouble(self.ask_volume),
                    JavaDouble(self.imp_volatility),
                    JavaDouble(self.open_interest),
                )
            }
        }

        time_series_fields! {
            [$($p).*]
            $cb! {
                head { $(#[$doc])* $name, $($head)* }
                fields {
                    /// The number of the original events (trades or quotes) in the candle.
                    count: i64 [$($p.)* count] = 0,
                    /// The first (open) price; NaN if unknown.
                    open: f64 [$($p.)* open] = f64::NAN,
                    /// The highest price; NaN if unknown.
                    high: f64 [$($p.)* high] = f64::NAN,
                    /// The lowest price; NaN if unknown.
                    low: f64 [$($p.)* low] = f64::NAN,
                    /// The last (close) price; NaN if unknown.
                    close: f64 [$($p.)* close] = f64::NAN,
                    /// The total volume; NaN if unknown.
                    volume: f64 [$($p.)* volume] = f64::NAN,
                    /// The volume-weighted average price; NaN if unknown.
                    #[doc(alias = "getVWAP")]
                    vwap: f64 [$($p.)* vwap] = f64::NAN,
                    /// The bid volume; NaN if unknown.
                    bid_volume: f64 [$($p.)* bid_volume] = f64::NAN,
                    /// The ask volume; NaN if unknown.
                    ask_volume: f64 [$($p.)* ask_volume] = f64::NAN,
                    /// The implied volatility; NaN if unknown.
                    imp_volatility: f64 [$($p.)* imp_volatility] = f64::NAN,
                    /// The open interest; NaN if unknown.
                    open_interest: f64 [$($p.)* open_interest] = f64::NAN,
                    $($fields)*
                }
                derived { $($derived)* }
            }
        }
    };
}

/// The fields, values and setters of Java's `OrderBase` at the path `[p]`, for [`event_type!`].
macro_rules! order_base_fields {
    (
        [$($p:ident).*]
        $cb:ident! {
            head { $(#[$doc:meta])* $name:ident, $($head:tt)* }
            fields { $($fields:tt)* }
            derived { $($derived:tt)* }
        }
    ) => {
        impl $name {
            /// Writes the text of Java's `OrderBase.baseFieldsToString` after `{`: the symbol to `tradeSize=...`.
            fn fmt_order_base(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                use $crate::event::format::{JavaChar, JavaDouble, SourceName, Time};
                write!(
                    f,
                    "{}, eventTime={}, source={}, eventFlags=0x{:x}, index=0x{:x}, time={}, sequence={}, \
                     timeNanoPart={}, action={}, actionTime={}, orderId={}, auxOrderId={}, price={}, size={}, \
                     executedSize={}, count={}, exchange={}, side={}, scope={}, tradeId={}, tradePrice={}, \
                     tradeSize={}",
                    self.symbol,
                    Time(self.event_time),
                    SourceName(self.source()),
                    self.event_flags,
                    self.index,
                    Time(self.time()),
                    self.sequence(),
                    self.time_nano_part,
                    self.action(),
                    Time(self.action_time),
                    self.order_id,
                    self.aux_order_id,
                    JavaDouble(self.price),
                    JavaDouble(self.size),
                    JavaDouble(self.executed_size),
                    self.count,
                    JavaChar(self.exchange_code()),
                    self.order_side(),
                    self.scope(),
                    self.trade_id,
                    JavaDouble(self.trade_price),
                    JavaDouble(self.trade_size),
                )
            }

            /// Changes the source of the order and keeps the low bits of the index, as Java's `setSource` does: a
            /// special source (`1` to `9`) goes to bits 48 to 63, any other to bits 32 to 63. Change the
            /// [`index`](Self::index) first: a new index replaces the source.
            ///
            /// # Errors
            ///
            /// [`Error::InvalidArgument`]($crate::Error::InvalidArgument) if Java has no source with the id: one to
            /// four letters and digits packed big-endian, such as `0x4e5456` for `NTV`, or `0` to `9`.
            #[doc(alias = "setSource")]
            pub fn set_source(&mut self, source: i32) -> $crate::Result<()> {
                if $crate::event::flags::source_name(source).is_none() {
                    return Err($crate::Error::InvalidArgument(format!("{source} is not an order source id")));
                }
                self.index = $crate::event::flags::with_source(self.index, source);
                Ok(())
            }

            /// Changes the time of the order and its nanoseconds part, as Java's `setTimeNanos` does.
            #[doc(alias = "setTimeNanos")]
            pub fn set_time_nanos(&mut self, time_nanos: i64) {
                let (time, nano_part) = $crate::event::flags::split_nanos(time_nanos);
                self.set_time(time);
                self.time_nano_part = nano_part;
            }

            /// Changes the action of the order.
            #[doc(alias = "setAction")]
            pub fn set_action(&mut self, action: $crate::event::OrderAction) {
                self.flags = $crate::event::flags::ORDER_ACTION.set32(self.flags, action.code());
            }

            /// Changes the exchange code of the order.
            ///
            /// # Errors
            ///
            /// [`Error::InvalidArgument`]($crate::Error::InvalidArgument) if the code is not an ASCII character: the
            /// flags keep 7 bits of it.
            #[doc(alias = "setExchangeCode")]
            pub fn set_exchange_code(&mut self, code: char) -> $crate::Result<()> {
                if !code.is_ascii() {
                    return Err($crate::Error::InvalidArgument(format!(
                        "exchange code {code:?} of an order is not ASCII"
                    )));
                }
                self.flags = $crate::event::flags::ORDER_EXCHANGE.set32(self.flags, code as i32);
                Ok(())
            }

            /// Changes the side of the order.
            #[doc(alias = "setOrderSide")]
            pub fn set_order_side(&mut self, side: $crate::event::Side) {
                self.flags = $crate::event::flags::ORDER_SIDE.set32(self.flags, side.code());
            }

            /// Changes the scope of the order.
            #[doc(alias = "setScope")]
            pub fn set_scope(&mut self, scope: $crate::event::Scope) {
                self.flags = $crate::event::flags::ORDER_SCOPE.set32(self.flags, scope.code());
            }
        }

        time_sequence_values! {
            time_sequence
            $cb! {
            head { $(#[$doc])* $name, $($head)* }
            fields {
                /// The flags of the event in a transaction or a snapshot.
                event_flags: event_flags [$($p.)* event_flags] = $crate::event::EventFlags::empty(),
                /// The index of the order among the orders of its symbol, with the source of the order in the high
                /// bits: see [`source`](Self::source).
                index: i64 [$($p.)* index] = 0,
                /// The time and the sequence of the order, packed as [`time`](Self::time) and
                /// [`sequence`](Self::sequence) read them.
                time_sequence: i64 [$($p.)* time_sequence] = 0,
                /// The microseconds and nanoseconds part of the time of the order.
                time_nano_part: i32 [$($p.)* time_nano_part] = 0,
                /// The time of the last action of the order, in milliseconds since the Unix epoch.
                action_time: i64 [$($p.)* action_time] = 0,
                /// The identifier of the order; `0` if unknown.
                order_id: i64 [$($p.)* order_id] = 0,
                /// The auxiliary identifier of the order, such as the order that it replaces; `0` if unknown.
                aux_order_id: i64 [$($p.)* aux_order_id] = 0,
                /// The price; NaN if unknown.
                price: f64 [$($p.)* price] = f64::NAN,
                /// The size; NaN if unknown.
                size: f64 [$($p.)* size] = f64::NAN,
                /// The executed size; NaN if unknown.
                executed_size: f64 [$($p.)* executed_size] = f64::NAN,
                /// The number of the individual orders at the price level.
                count: i64 [$($p.)* count] = 0,
                /// The flags of the order, packed as the SDK keeps them: the action in bits 11 to 14, the exchange
                /// code in bits 4 to 10, the side in bits 2 and 3, the scope in bits 0 and 1.
                flags: i32 [$($p.)* flags] = 0,
                /// The identifier of the trade of an execution; `0` if none.
                trade_id: i64 [$($p.)* trade_id] = 0,
                /// The price of the trade of an execution; NaN if unknown.
                trade_price: f64 [$($p.)* trade_price] = f64::NAN,
                /// The size of the trade of an execution; NaN if unknown.
                trade_size: f64 [$($p.)* trade_size] = f64::NAN,
                $($fields)*
            }
            derived {
                /// The id of the source of the order, from the high bits of the [`index`](Self::index): `0` for the
                /// default source `DEFAULT`, `1` to `9` for the special sources (`COMPOSITE_BID` to `AGGREGATE`), or
                /// one to four letters and digits packed big-endian, such as `0x4e5456` for `NTV`. A later version
                /// returns the source itself.
                #[doc(alias = "getSource")]
                fn source(index) -> i32 {
                    $crate::event::flags::source_of(index)
                }

                /// The time of the order, in nanoseconds since the Unix epoch.
                #[doc(alias = "getTimeNanos")]
                fn time_nanos(time_sequence, time_nano_part) -> i64 {
                    $crate::event::flags::nanos($crate::event::flags::time_of(time_sequence), time_nano_part)
                }

                /// The action of the order.
                #[doc(alias = "getAction")]
                fn action(flags) -> $crate::event::OrderAction {
                    $crate::event::OrderAction::from_code($crate::event::flags::ORDER_ACTION.get32(flags))
                }

                /// The exchange code of the order; `'\0'` if unknown.
                #[doc(alias = "getExchangeCode")]
                fn exchange_code(flags) -> char {
                    char::from($crate::event::flags::ORDER_EXCHANGE.get32(flags) as u8)
                }

                /// The side of the order.
                #[doc(alias = "getOrderSide")]
                fn order_side(flags) -> $crate::event::Side {
                    $crate::event::Side::from_code($crate::event::flags::ORDER_SIDE.get32(flags))
                }

                /// The scope of the order: composite, regional, aggregate or an individual order.
                #[doc(alias = "getScope")]
                fn scope(flags) -> $crate::event::Scope {
                    $crate::event::Scope::from_code($crate::event::flags::ORDER_SCOPE.get32(flags))
                }

                $($derived)*
            }
            }
        }
    };
}
