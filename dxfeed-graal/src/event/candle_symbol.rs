//! The symbols of candles and their attributes (`CandleSymbol`, `CandlePeriod`, `CandleType`, `CandlePrice`,
//! `CandleSession`, `CandleAlignment`, `CandlePriceLevel`, `CandleExchange`, `CandleDataType`).

use std::borrow::Cow;
use std::fmt::{self, Display, Formatter};
use std::hash::{Hash, Hasher};
use std::ptr::{self, NonNull};
use std::str::FromStr;

use dxfeed_graal_sys as sys;

use super::format::JavaDouble;
use super::market_event_symbols::{
    change_attribute_string_by_key, change_exchange_code, get_attribute_string_by_key, get_exchange_code,
    remove_attribute_string_by_key,
};
use crate::error::{Error, JavaException, Result, c_string};
use crate::isolate::{self, call};
use crate::schedule::SessionFilter;

/// The symbol of candles: a base symbol with the attributes of the candles in braces, such as `AAPL{=d}` for the
/// daily candles of `AAPL`, or `AAPL&Q{=5m,price=bid}` for the 5-minute candles of its bid prices on the exchange `Q`
/// (`CandleSymbol`).
///
/// A candle symbol is normalized as Java's `CandleSymbol.valueOf` normalizes it: the attributes with their default
/// values are removed, and a new attribute goes before the first one whose key is greater, as Java adds it.
///
/// ```no_run
/// # fn main() -> dxfeed_graal::Result<()> {
/// use dxfeed_graal::event::{CandlePeriod, CandleSymbol};
///
/// assert_eq!(CandleSymbol::value_of("AAPL{=1d}")?.as_str(), "AAPL{=d}");
/// assert_eq!(CandleSymbol::value_of_with("AAPL", [CandlePeriod::DAY])?.as_str(), "AAPL{=d}");
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CandleSymbol(String);

impl CandleSymbol {
    /// The normalized candle symbol of the text `symbol` (`CandleSymbol.valueOf(String)`).
    ///
    /// Returns [`Error::Java`] for a malformed attribute, such as `AAPL{=zzz}`.
    #[doc(alias = "valueOf")]
    pub fn value_of(symbol: &str) -> Result<CandleSymbol> {
        normalize(symbol).map(CandleSymbol)
    }

    /// The normalized candle symbol of `symbol` with `attributes`, which replace those of the same kinds in `symbol`
    /// (`CandleSymbol.valueOf(String, CandleSymbolAttribute...)`).
    ///
    /// Returns [`Error::Java`] for a malformed attribute.
    #[doc(alias = "valueOf")]
    pub fn value_of_with(
        symbol: &str,
        attributes: impl IntoIterator<Item: Into<CandleAttribute>>,
    ) -> Result<CandleSymbol> {
        let mut text = symbol.to_owned();
        for attribute in attributes {
            text = attribute.into().change_attribute_for_symbol(&text);
        }
        normalize(&text).map(CandleSymbol)
    }

    /// The candle symbol of a text the SDK has normalized.
    pub(crate) fn from_normalized(text: String) -> CandleSymbol {
        CandleSymbol(text)
    }

    /// The text of the symbol (`toString`).
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for CandleSymbol {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.pad(&self.0)
    }
}

impl AsRef<str> for CandleSymbol {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl FromStr for CandleSymbol {
    type Err = Error;

    /// Parses a candle symbol as [`CandleSymbol::value_of`] does.
    fn from_str(symbol: &str) -> Result<Self> {
        CandleSymbol::value_of(symbol)
    }
}

/// An attribute of a candle symbol, for [`CandleSymbol::value_of_with`] (`CandleSymbolAttribute`).
#[derive(Debug, Clone, PartialEq)]
pub enum CandleAttribute {
    /// The exchange of the trades of the candles, as `&Q` after the base symbol (`CandleExchange`).
    Exchange(CandleExchange),
    /// The aggregation period, as `=5m` (`CandlePeriod`).
    Period(CandlePeriod),
    /// The price the candles aggregate, as `price=bid` (`CandlePrice`).
    Price(CandlePrice),
    /// The trading session of the candles, as `tho=true` (`CandleSession`).
    Session(CandleSession),
    /// The alignment of the candles, as `a=s` (`CandleAlignment`).
    Alignment(CandleAlignment),
    /// The price level of the candles of the price types, as `pl=0.5` (`CandlePriceLevel`).
    PriceLevel(CandlePriceLevel),
    /// The type of the data of the candles, as `dt=tho` (`CandleDataType`).
    DataType(CandleDataType),
}

impl CandleAttribute {
    /// `symbol` with this attribute, as Java's `changeAttributeForSymbol` writes it: a default value removes the
    /// attribute, another value replaces that of the key in place or goes before the first attribute whose key is
    /// greater; the exchange replaces the exchange code.
    #[doc(alias = "changeAttributeForSymbol")]
    pub fn change_attribute_for_symbol(&self, symbol: &str) -> String {
        let (key, value) = match self {
            CandleAttribute::Exchange(exchange) => return change_exchange_code(symbol, exchange.get_exchange_code()),
            CandleAttribute::Period(period) => (
                CandlePeriod::ATTRIBUTE_KEY,
                (*period != CandlePeriod::DEFAULT).then(|| period.to_string()),
            ),
            CandleAttribute::Price(price) => (
                CandlePrice::ATTRIBUTE_KEY,
                (*price != CandlePrice::DEFAULT).then(|| price.to_string()),
            ),
            CandleAttribute::Session(session) => (
                CandleSession::ATTRIBUTE_KEY,
                (*session != CandleSession::DEFAULT).then(|| session.to_string()),
            ),
            CandleAttribute::Alignment(alignment) => (
                CandleAlignment::ATTRIBUTE_KEY,
                (*alignment != CandleAlignment::DEFAULT).then(|| alignment.to_string()),
            ),
            CandleAttribute::PriceLevel(level) => (
                CandlePriceLevel::ATTRIBUTE_KEY,
                (*level != CandlePriceLevel::DEFAULT).then(|| level.to_string()),
            ),
            CandleAttribute::DataType(data_type) => (
                CandleDataType::ATTRIBUTE_KEY,
                (*data_type != CandleDataType::DEFAULT).then(|| data_type.to_string()),
            ),
        };
        match value {
            Some(value) => change_attribute_string_by_key(symbol, key, Some(&value)),
            None => remove_attribute_string_by_key(symbol, key),
        }
    }
}

/// Declares the conversion of an attribute type into [`CandleAttribute`] and its `changeAttributeForSymbol`.
macro_rules! candle_attribute {
    ($($name:ident => $variant:ident,)*) => {
        $(
            impl From<$name> for CandleAttribute {
                fn from(value: $name) -> Self {
                    CandleAttribute::$variant(value)
                }
            }

            impl $name {
                /// `symbol` with this attribute, as [`CandleAttribute::change_attribute_for_symbol`] writes it.
                #[doc(alias = "changeAttributeForSymbol")]
                pub fn change_attribute_for_symbol(&self, symbol: &str) -> String {
                    CandleAttribute::from(Clone::clone(self)).change_attribute_for_symbol(symbol)
                }
            }
        )*
    };
}

candle_attribute! {
    CandleExchange => Exchange,
    CandlePeriod => Period,
    CandlePrice => Price,
    CandleSession => Session,
    CandleAlignment => Alignment,
    CandlePriceLevel => PriceLevel,
    CandleDataType => DataType,
}

/// The aggregation period of candles: a number of units of a [`CandleType`], such as 5 minutes (`CandlePeriod`).
///
/// Two periods are equal when their types are and their values are equal as Java's `Double.compare` sees them: `NaN`
/// equals `NaN`, `0.0` does not equal `-0.0`.
#[derive(Debug, Clone, Copy)]
pub struct CandlePeriod {
    value: f64,
    candle_type: CandleType,
}

impl CandlePeriod {
    /// One tick: the default period, each trade is a candle (`CandlePeriod.TICK`).
    pub const TICK: CandlePeriod = CandlePeriod::value_of(1.0, CandleType::Tick);
    /// One day (`CandlePeriod.DAY`).
    pub const DAY: CandlePeriod = CandlePeriod::value_of(1.0, CandleType::Day);
    /// The default period, [`TICK`](Self::TICK) (`CandlePeriod.DEFAULT`).
    pub const DEFAULT: CandlePeriod = CandlePeriod::TICK;
    /// The key of the period in a candle symbol: the empty string, as in `AAPL{=5m}` (`CandlePeriod.ATTRIBUTE_KEY`).
    pub const ATTRIBUTE_KEY: &'static str = "";

    /// The period of `value` units of `candle_type` (`CandlePeriod.valueOf`).
    #[doc(alias = "valueOf")]
    pub const fn value_of(value: f64, candle_type: CandleType) -> CandlePeriod {
        CandlePeriod { value, candle_type }
    }

    /// The number of units.
    #[doc(alias = "getValue")]
    pub fn get_value(self) -> f64 {
        self.value
    }

    /// The unit.
    #[doc(alias = "getType")]
    pub fn get_type(self) -> CandleType {
        self.candle_type
    }
}

impl Default for CandlePeriod {
    fn default() -> Self {
        CandlePeriod::DEFAULT
    }
}

impl PartialEq for CandlePeriod {
    fn eq(&self, other: &Self) -> bool {
        java_bits(self.value) == java_bits(other.value) && self.candle_type == other.candle_type
    }
}

impl Eq for CandlePeriod {}

impl Hash for CandlePeriod {
    fn hash<H: Hasher>(&self, state: &mut H) {
        java_bits(self.value).hash(state);
        self.candle_type.hash(state);
    }
}

impl Display for CandlePeriod {
    /// Prints the period as Java does: the unit alone for one unit, such as `d`, else the number and the unit, such as
    /// `5m`.
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        if self.value == 1.0 {
            write!(f, "{}", self.candle_type)
        } else {
            write!(f, "{}{}", JavaNumber(self.value), self.candle_type)
        }
    }
}

/// The bits of a double that Java's `Double.compare` and `Double.hashCode` see: all NaNs are one.
fn java_bits(value: f64) -> u64 {
    if value.is_nan() {
        f64::NAN.to_bits()
    } else {
        value.to_bits()
    }
}

/// A number as Java prints the values of candle attributes: a whole number that fits a `long` without a fraction, any
/// other as `Double.toString` prints it.
struct JavaNumber(f64);

impl Display for JavaNumber {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let value = self.0;
        if (value as i64) as f64 == value {
            write!(f, "{}", value as i64)
        } else {
            write!(f, "{}", JavaDouble(value))
        }
    }
}

/// The exchange of the trades of candles, as `&Q` after the base symbol; [`COMPOSITE`](Self::COMPOSITE) for the
/// composite of all exchanges, the default (`CandleExchange`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CandleExchange(char);

impl CandleExchange {
    /// The composite of all exchanges, without an exchange code (`CandleExchange.COMPOSITE`).
    pub const COMPOSITE: CandleExchange = CandleExchange('\0');
    /// The default exchange, [`COMPOSITE`](Self::COMPOSITE) (`CandleExchange.DEFAULT`).
    pub const DEFAULT: CandleExchange = CandleExchange::COMPOSITE;

    /// The exchange with the code `exchange_code`; `'\0'` gives [`COMPOSITE`](Self::COMPOSITE)
    /// (`CandleExchange.valueOf`).
    #[doc(alias = "valueOf")]
    pub const fn value_of(exchange_code: char) -> CandleExchange {
        CandleExchange(exchange_code)
    }

    /// The exchange code; `'\0'` for [`COMPOSITE`](Self::COMPOSITE).
    #[doc(alias = "getExchangeCode")]
    pub fn get_exchange_code(self) -> char {
        self.0
    }

    /// The exchange of `symbol`, from its exchange code (`CandleExchange.getAttributeForSymbol`).
    #[doc(alias = "getAttributeForSymbol")]
    pub fn get_attribute_for_symbol(symbol: &str) -> CandleExchange {
        CandleExchange(get_exchange_code(symbol))
    }
}

impl Default for CandleExchange {
    fn default() -> Self {
        CandleExchange::DEFAULT
    }
}

impl Display for CandleExchange {
    /// Prints `COMPOSITE` or the exchange code, as Java's `toString`.
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self.0 {
            '\0' => f.pad("COMPOSITE"),
            code => f.pad(code.encode_utf8(&mut [0; 4])),
        }
    }
}

/// The price level of the candles of the price types, as `pl=0.5`; [`DEFAULT`](Self::DEFAULT) (`NaN`) for none
/// (`CandlePriceLevel`).
///
/// The value is never negative nor infinite; levels are equal as Java's `Double.compare` sees them.
#[derive(Debug, Clone, Copy)]
pub struct CandlePriceLevel(f64);

impl CandlePriceLevel {
    /// No price level, `NaN` (`CandlePriceLevel.DEFAULT`).
    pub const DEFAULT: CandlePriceLevel = CandlePriceLevel(f64::NAN);
    /// The key of the price level in a candle symbol (`CandlePriceLevel.ATTRIBUTE_KEY`).
    pub const ATTRIBUTE_KEY: &'static str = "pl";

    /// The price level `value`; `NaN` gives [`DEFAULT`](Self::DEFAULT) (`CandlePriceLevel.valueOf`).
    ///
    /// Returns [`Error::InvalidArgument`] for a negative value, `-0.0` included, or an infinite one.
    #[doc(alias = "valueOf")]
    pub fn value_of(value: f64) -> Result<CandlePriceLevel> {
        if value.is_infinite() || value.is_sign_negative() && !value.is_nan() {
            return Err(Error::InvalidArgument(format!(
                "Incorrect candle price level: {}",
                JavaDouble(value)
            )));
        }
        Ok(CandlePriceLevel(value))
    }

    /// Parses a price level as Java's `Double.parseDouble` reads a number, such as `0.5` or `1E3`
    /// (`CandlePriceLevel.parse`); the hexadecimal form is not read.
    ///
    /// Returns [`Error::InvalidArgument`] for any other text and the values [`value_of`](Self::value_of) refuses.
    pub fn parse(text: &str) -> Result<CandlePriceLevel> {
        let value = parse_java_double(text).ok_or_else(|| {
            let text = text.trim_matches(|c| c <= ' ');
            Error::InvalidArgument(if text.is_empty() {
                "empty String".to_owned()
            } else {
                format!("For input string: \"{text}\"")
            })
        })?;
        CandlePriceLevel::value_of(value)
    }

    /// The value; `NaN` for [`DEFAULT`](Self::DEFAULT).
    #[doc(alias = "getValue")]
    pub fn get_value(self) -> f64 {
        self.0
    }

    /// The price level of `symbol`, [`DEFAULT`](Self::DEFAULT) without one (`CandlePriceLevel.getAttributeForSymbol`).
    ///
    /// Returns [`Error::InvalidArgument`] for a value that [`parse`](Self::parse) refuses.
    #[doc(alias = "getAttributeForSymbol")]
    pub fn get_attribute_for_symbol(symbol: &str) -> Result<CandlePriceLevel> {
        get_attribute_string_by_key(symbol, Self::ATTRIBUTE_KEY).map_or(Ok(Self::DEFAULT), Self::parse)
    }

    /// `symbol` with the price level written as Java writes it, such as `pl=1` for `pl=1.0`; a malformed or default
    /// value is kept, as in Java (`CandlePriceLevel.normalizeAttributeForSymbol`).
    #[doc(alias = "normalizeAttributeForSymbol")]
    pub fn normalize_attribute_for_symbol(symbol: &str) -> String {
        normalize_attribute(symbol, Self::ATTRIBUTE_KEY, |text| {
            Self::parse(text).ok().map(|level| level.to_string())
        })
    }
}

impl Default for CandlePriceLevel {
    fn default() -> Self {
        CandlePriceLevel::DEFAULT
    }
}

impl PartialEq for CandlePriceLevel {
    fn eq(&self, other: &Self) -> bool {
        java_bits(self.0) == java_bits(other.0)
    }
}

impl Eq for CandlePriceLevel {}

impl Hash for CandlePriceLevel {
    fn hash<H: Hasher>(&self, state: &mut H) {
        java_bits(self.0).hash(state);
    }
}

impl Display for CandlePriceLevel {
    /// Prints the value as Java's `toString`: `1` for `1.0`, `0.5`, `NaN`.
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", JavaNumber(self.0))
    }
}

impl FromStr for CandlePriceLevel {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self> {
        CandlePriceLevel::parse(text)
    }
}

/// A number as Java's `Double.parseDouble` reads it: surrounding control characters and spaces are ignored, a
/// trailing `d` or `f` is allowed, and the special values are written `NaN` and `Infinity`.
fn parse_java_double(text: &str) -> Option<f64> {
    let text = text.trim_matches(|c| c <= ' ');
    let unsigned = text.strip_prefix(['+', '-']).unwrap_or(text);
    if unsigned.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return match unsigned {
            "NaN" => Some(f64::NAN),
            "Infinity" if text.starts_with('-') => Some(f64::NEG_INFINITY),
            "Infinity" => Some(f64::INFINITY),
            _ => None,
        };
    }
    let number = text.strip_suffix(['d', 'D', 'f', 'F']).unwrap_or(text);
    number.parse().ok()
}

/// The type of the data of candles, as `dt=tho`; [`UNDEFINED`](Self::UNDEFINED), the default, for none
/// (`CandleDataType`). Any other string is a type too.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CandleDataType(Cow<'static, str>);

impl CandleDataType {
    /// The data of the source (`CandleDataType.SOURCE`).
    pub const SOURCE: CandleDataType = CandleDataType(Cow::Borrowed("source"));
    /// The data of the trading hours only (`CandleDataType.THO`).
    pub const THO: CandleDataType = CandleDataType(Cow::Borrowed("tho"));
    /// All the data (`CandleDataType.ALL`).
    pub const ALL: CandleDataType = CandleDataType(Cow::Borrowed("all"));
    /// The data of the regular trading hours (`CandleDataType.RTH`).
    pub const RTH: CandleDataType = CandleDataType(Cow::Borrowed("rth"));
    /// The data of the extended trading hours (`CandleDataType.ETH`).
    pub const ETH: CandleDataType = CandleDataType(Cow::Borrowed("eth"));
    /// No data type, the empty string (`CandleDataType.UNDEFINED`).
    pub const UNDEFINED: CandleDataType = CandleDataType(Cow::Borrowed(""));
    /// The default data type, [`UNDEFINED`](Self::UNDEFINED) (`CandleDataType.DEFAULT`).
    pub const DEFAULT: CandleDataType = CandleDataType::UNDEFINED;
    /// The key of the data type in a candle symbol (`CandleDataType.ATTRIBUTE_KEY`).
    pub const ATTRIBUTE_KEY: &'static str = "dt";

    /// The data type of the string `value`: one of the constants for its string, the empty string included, or a
    /// new type (`CandleDataType.valueOf`).
    #[doc(alias = "valueOf")]
    pub fn value_of(value: &str) -> CandleDataType {
        [
            Self::SOURCE,
            Self::THO,
            Self::ALL,
            Self::RTH,
            Self::ETH,
            Self::UNDEFINED,
        ]
        .into_iter()
        .find(|known| known.0 == value)
        .unwrap_or_else(|| CandleDataType(Cow::Owned(value.to_owned())))
    }

    /// The data type of the string `value`, as [`value_of`](Self::value_of) (`CandleDataType.parse`).
    pub fn parse(value: &str) -> CandleDataType {
        CandleDataType::value_of(value)
    }

    /// The string of the data type, such as `tho` (`getValue`).
    #[doc(alias = "getValue")]
    pub fn get_value(&self) -> &str {
        &self.0
    }

    /// The data type of `symbol`, [`DEFAULT`](Self::DEFAULT) without one (`CandleDataType.getAttributeForSymbol`).
    #[doc(alias = "getAttributeForSymbol")]
    pub fn get_attribute_for_symbol(symbol: &str) -> CandleDataType {
        get_attribute_string_by_key(symbol, Self::ATTRIBUTE_KEY).map_or(Self::DEFAULT, Self::parse)
    }

    /// `symbol` without an empty data type, as Java's `CandleDataType.normalizeAttributeForSymbol`.
    #[doc(alias = "normalizeAttributeForSymbol")]
    pub fn normalize_attribute_for_symbol(symbol: &str) -> String {
        match get_attribute_string_by_key(symbol, Self::ATTRIBUTE_KEY) {
            Some("") => remove_attribute_string_by_key(symbol, Self::ATTRIBUTE_KEY),
            _ => symbol.to_owned(),
        }
    }
}

impl Default for CandleDataType {
    fn default() -> Self {
        CandleDataType::DEFAULT
    }
}

impl Display for CandleDataType {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.pad(&self.0)
    }
}

impl FromStr for CandleDataType {
    type Err = Error;

    /// Parses a data type as [`CandleDataType::value_of`] does; it never fails.
    fn from_str(value: &str) -> Result<Self> {
        Ok(CandleDataType::value_of(value))
    }
}

/// `symbol` with the value of `key` replaced by the text that `normalized` gives, as the Java attributes normalize a
/// symbol: a value that `normalized` refuses, or that it does not change, keeps the symbol.
fn normalize_attribute(symbol: &str, key: &str, normalized: impl FnOnce(&str) -> Option<String>) -> String {
    match get_attribute_string_by_key(symbol, key).and_then(|text| Some((text, normalized(text)?))) {
        Some((text, value)) if text != value => change_attribute_string_by_key(symbol, key, Some(&value)),
        _ => symbol.to_owned(),
    }
}

/// Declares an enum of a candle attribute with the Java names and strings of its values, and its parsing by `$parse`.
macro_rules! candle_enum {
    (
        $(#[$meta:meta])* $name:ident, $what:literal, $parse:ident {
            $($(#[$vmeta:meta])* $variant:ident = ($java:literal, $string:literal),)*
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name {
            $( $(#[$vmeta])* $variant, )*
        }

        impl $name {
            /// Every value, in the order of Java's `values()`.
            #[doc(alias = "values")]
            pub const ALL: &'static [$name] = &[ $( $name::$variant, )* ];

            const VALUES: &'static [($name, &'static str, &'static str)] = &[ $( ($name::$variant, $java, $string), )* ];

            /// The string of the value in a candle symbol, such as `bid` (`toString`).
            pub fn as_str(self) -> &'static str {
                match self {
                    $( $name::$variant => $string, )*
                }
            }

            /// Parses a value as Java's `parse` does; see the type for the accepted texts.
            ///
            /// Returns [`Error::InvalidArgument`] for any other text.
            pub fn parse(text: &str) -> Result<$name> {
                $parse(text, Self::VALUES, $what)
            }
        }

        impl Display for $name {
            fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
                f.pad(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = Error;

            fn from_str(text: &str) -> Result<Self> {
                $name::parse(text)
            }
        }
    };
}

/// Declares the `DEFAULT`, `ATTRIBUTE_KEY`, `getAttributeForSymbol` and `normalizeAttributeForSymbol` of an enum
/// attribute whose Java code reads the value with `parse`.
macro_rules! parsed_attribute {
    ($name:ident, $default:ident, $key:literal) => {
        impl $name {
            /// The default value (`DEFAULT`).
            pub const DEFAULT: $name = $name::$default;
            /// The key of the attribute in a candle symbol (`ATTRIBUTE_KEY`).
            pub const ATTRIBUTE_KEY: &'static str = $key;

            /// The value of the attribute in `symbol`, [`DEFAULT`](Self::DEFAULT) without one (`getAttributeForSymbol`).
            ///
            /// Returns [`Error::InvalidArgument`] for a value that [`parse`](Self::parse) refuses.
            #[doc(alias = "getAttributeForSymbol")]
            pub fn get_attribute_for_symbol(symbol: &str) -> Result<$name> {
                get_attribute_string_by_key(symbol, Self::ATTRIBUTE_KEY).map_or(Ok(Self::DEFAULT), Self::parse)
            }

            /// `symbol` with the value of the attribute written as [`as_str`](Self::as_str) gives it; a malformed
            /// value is kept, and so is a default one, as in Java (`normalizeAttributeForSymbol`).
            #[doc(alias = "normalizeAttributeForSymbol")]
            pub fn normalize_attribute_for_symbol(symbol: &str) -> String {
                normalize_attribute(symbol, Self::ATTRIBUTE_KEY, |text| {
                    Self::parse(text).ok().map(|value| value.as_str().to_owned())
                })
            }
        }

        impl Default for $name {
            fn default() -> Self {
                $name::DEFAULT
            }
        }
    };
}

candle_enum!(
    /// The unit of a [`CandlePeriod`] (`CandleType`).
    ///
    /// [`parse`](Self::parse) takes the string of a value, such as `m`, else the start of a Java name in any case,
    /// such as `Min`, else a plural that is a whole Java name, such as `Minutes`.
    CandleType, "candle type", parse_type {
        /// Ticks: each trade is a candle (`t`).
        Tick = ("TICK", "t"),
        /// Seconds (`s`).
        Second = ("SECOND", "s"),
        /// Minutes (`m`).
        Minute = ("MINUTE", "m"),
        /// Hours (`h`).
        Hour = ("HOUR", "h"),
        /// Days (`d`).
        Day = ("DAY", "d"),
        /// Weeks (`w`).
        Week = ("WEEK", "w"),
        /// Months (`mo`).
        Month = ("MONTH", "mo"),
        /// Option expirations, the third Friday of a month (`o`).
        OptExp = ("OPTEXP", "o"),
        /// Years (`y`).
        Year = ("YEAR", "y"),
        /// Volume: each candle aggregates the volume of the period value (`v`).
        Volume = ("VOLUME", "v"),
        /// Price: each candle spans the price range of the period value (`p`).
        Price = ("PRICE", "p"),
        /// Price momentum (`pm`).
        PriceMomentum = ("PRICE_MOMENTUM", "pm"),
        /// Price renko (`pr`).
        PriceRenko = ("PRICE_RENKO", "pr"),
    }
);

candle_enum!(
    /// The price that candles aggregate (`CandlePrice`); [`Last`](Self::Last) is the default.
    ///
    /// [`parse`](Self::parse) takes the string of a value, such as `bid`, or its start in any case, such as `BI`.
    CandlePrice, "candle price", parse_string_prefix {
        /// The prices of the trades (`last`).
        Last = ("LAST", "last"),
        /// The bid prices (`bid`).
        Bid = ("BID", "bid"),
        /// The ask prices (`ask`).
        Ask = ("ASK", "ask"),
        /// The middles of the bid and ask prices (`mark`).
        Mark = ("MARK", "mark"),
        /// The official settlement prices (`s`).
        Settlement = ("SETTLEMENT", "s"),
    }
);

parsed_attribute!(CandlePrice, Last, "price");

candle_enum!(
    /// The trading session of candles (`CandleSession`); [`Any`](Self::Any) is the default.
    ///
    /// [`parse`](Self::parse) takes the string of a value, `false` or `true`, or its start in any case, such as `t`.
    CandleSession, "candle session", parse_string_prefix {
        /// All the trading sessions (`false`).
        Any = ("ANY", "false"),
        /// The regular trading session only (`true`).
        Regular = ("REGULAR", "true"),
    }
);

impl CandleSession {
    /// The default session, [`Any`](Self::Any) (`CandleSession.DEFAULT`).
    pub const DEFAULT: CandleSession = CandleSession::Any;
    /// The key of the session in a candle symbol (`CandleSession.ATTRIBUTE_KEY`).
    pub const ATTRIBUTE_KEY: &'static str = "tho";

    /// The filter of the trading sessions of the candles (`getSessionFilter`).
    #[doc(alias = "getSessionFilter")]
    pub fn get_session_filter(self) -> SessionFilter {
        match self {
            CandleSession::Any => SessionFilter::Any,
            CandleSession::Regular => SessionFilter::Regular,
        }
    }

    /// The session of `symbol`: [`Regular`](Self::Regular) if its `tho` attribute is `true` in any case, else
    /// [`DEFAULT`](Self::DEFAULT) (`CandleSession.getAttributeForSymbol`).
    #[doc(alias = "getAttributeForSymbol")]
    pub fn get_attribute_for_symbol(symbol: &str) -> CandleSession {
        match get_attribute_string_by_key(symbol, Self::ATTRIBUTE_KEY) {
            Some(text) if text.eq_ignore_ascii_case("true") => CandleSession::Regular,
            _ => CandleSession::DEFAULT,
        }
    }

    /// `symbol` with `tho=true` for a `tho` attribute that is `true` in another case; any other value is kept, as in
    /// Java (`CandleSession.normalizeAttributeForSymbol`).
    #[doc(alias = "normalizeAttributeForSymbol")]
    pub fn normalize_attribute_for_symbol(symbol: &str) -> String {
        normalize_attribute(symbol, Self::ATTRIBUTE_KEY, |text| {
            text.eq_ignore_ascii_case("true").then(|| "true".to_owned())
        })
    }
}

impl Default for CandleSession {
    fn default() -> Self {
        CandleSession::DEFAULT
    }
}

candle_enum!(
    /// The alignment of candles (`CandleAlignment`); [`Midnight`](Self::Midnight) is the default.
    ///
    /// [`parse`](Self::parse) takes the string of a value, `m` or `s`, in any case.
    CandleAlignment, "candle alignment", parse_string_ignoring_case {
        /// Aligned to the midnight (`m`).
        Midnight = ("MIDNIGHT", "m"),
        /// Aligned to the start of the trading session (`s`).
        Session = ("SESSION", "s"),
    }
);

parsed_attribute!(CandleAlignment, Midnight, "a");

/// Tells whether `a` and `b` are equal ignoring case, as Java's `String.equalsIgnoreCase` compares them: char by char,
/// the upper cases or the lower cases of the upper cases equal.
fn java_equals_ignore_case(a: &str, b: &str) -> bool {
    /// The only char of `chars`, else `c`, as Java's `Character.toUpperCase` keeps a char without a single upper case.
    fn single(mut chars: impl Iterator<Item = char>, c: char) -> char {
        match (chars.next(), chars.next()) {
            (Some(single), None) => single,
            _ => c,
        }
    }
    let upper = |c: char| single(c.to_uppercase(), c);
    // `İ` is the only char whose full lower case has more chars while its simple one, which Java takes, is `i`.
    let lower = |c: char| {
        if c == '\u{130}' {
            'i'
        } else {
            single(c.to_lowercase(), c)
        }
    };
    a.encode_utf16().count() == b.encode_utf16().count()
        && a.chars()
            .zip(b.chars())
            .all(|(x, y)| x == y || upper(x) == upper(y) || lower(upper(x)) == lower(upper(y)))
}

/// The first `n` UTF-16 units of the ASCII `name`, as Java's `substring(0, n)`; `None` if it is shorter.
fn java_prefix(name: &str, n: usize) -> Option<&str> {
    name.get(..n)
}

/// Java's `CandleType.parse`: the string of a value, else for each value in order, the start of its Java name in any
/// case or a plural that is its Java name in any case.
fn parse_type<T: Copy>(text: &str, values: &[(T, &str, &str)], what: &str) -> Result<T> {
    let n = text.encode_utf16().count();
    if n == 0 {
        return Err(missing(what));
    }
    values
        .iter()
        .find(|(_, _, string)| *string == text)
        .or_else(|| {
            values.iter().find(|(_, name, _)| {
                java_prefix(name, n).is_some_and(|prefix| java_equals_ignore_case(prefix, text))
                    || text
                        .strip_suffix('s')
                        .is_some_and(|singular| java_equals_ignore_case(name, singular))
            })
        })
        .map(|(value, _, _)| *value)
        .ok_or_else(|| unknown(what, text))
}

/// Java's `CandlePrice.parse` and `CandleSession.parse`: the string of a value, or its start in any case.
fn parse_string_prefix<T: Copy>(text: &str, values: &[(T, &str, &str)], what: &str) -> Result<T> {
    let n = text.encode_utf16().count();
    if n == 0 {
        return Err(missing(what));
    }
    values
        .iter()
        .find(|(_, _, string)| *string == text)
        .or_else(|| {
            values.iter().find(|(_, _, string)| {
                java_prefix(string, n).is_some_and(|prefix| java_equals_ignore_case(prefix, text))
            })
        })
        .map(|(value, _, _)| *value)
        .ok_or_else(|| unknown(what, text))
}

/// Java's `CandleAlignment.parse`: the string of a value in any case.
fn parse_string_ignoring_case<T: Copy>(text: &str, values: &[(T, &str, &str)], what: &str) -> Result<T> {
    values
        .iter()
        .find(|(_, _, string)| *string == text)
        .or_else(|| {
            values
                .iter()
                .find(|(_, _, string)| java_equals_ignore_case(string, text))
        })
        .map(|(value, _, _)| *value)
        .ok_or_else(|| unknown(what, text))
}

/// The error of an empty attribute value, as Java's `Missing candle price`.
fn missing(what: &str) -> Error {
    Error::InvalidArgument(format!("Missing {what}"))
}

/// The error of an unknown attribute value, as Java's `Unknown candle price: zzz`.
fn unknown(what: &str, text: &str) -> Error {
    Error::InvalidArgument(format!("Unknown {what}: {text}"))
}

/// The text of the candle symbol `symbol` normalized by the SDK (Java `CandleSymbol.valueOf`).
fn normalize(symbol: &str) -> Result<String> {
    let text = c_string(symbol, "the candle symbol")?;
    let mut raw = ptr::null_mut();
    // SAFETY: the text is NUL-terminated and the out-pointer is valid for writes.
    call(|t| unsafe { sys::dxfg_v4_Symbol_new(t, text.as_ptr(), sys::CANDLE, &mut raw) })?;
    let symbol = NonNull::new(raw).ok_or_else(|| Error::Java(JavaException::null_result()))?;
    // SAFETY: the symbol is live until it is released below; a candle symbol is a `dxfg_candle_symbol_t`.
    let normalized = unsafe {
        let symbol = symbol.as_ptr();
        ((*symbol).type_ == sys::CANDLE)
            .then(|| (*symbol.cast::<sys::dxfg_candle_symbol_t>()).symbol)
            .filter(|text| !text.is_null())
            .map(|text| isolate::string(text).into_owned())
    };
    // SAFETY: the symbol came from dxfg_v4_Symbol_new and is released once; a failed release leaks it.
    crate::error_manager::report_release(
        "CandleSymbol",
        call(|t| unsafe { sys::dxfg_v4_Symbol_release(t, symbol.as_ptr()) }),
    );
    normalized.ok_or_else(|| Error::Java(JavaException::from_message("the SDK returned no candle symbol")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `symbol` with `attribute`.
    fn change(attribute: impl Into<CandleAttribute>, symbol: &str) -> String {
        attribute.into().change_attribute_for_symbol(symbol)
    }

    #[test]
    fn attributes_change_the_text() {
        let period = CandlePeriod::value_of(5.0, CandleType::Minute);
        assert_eq!(change(period, "AAPL"), "AAPL{=5m}");
        assert_eq!(change(period, "AAPL{=d,price=bid}"), "AAPL{=5m,price=bid}");
        assert_eq!(
            period.change_attribute_for_symbol("AAPL{price=bid}"),
            "AAPL{=5m,price=bid}"
        );
        assert_eq!(
            change(CandlePrice::Bid, "AAPL{=d,tho=true}"),
            "AAPL{=d,price=bid,tho=true}"
        );
        assert_eq!(change(CandleSession::Regular, "AAPL{=d}"), "AAPL{=d,tho=true}");
        assert_eq!(change(CandleExchange::value_of('Q'), "AAPL&N{=d}"), "AAPL&Q{=d}");
        assert_eq!(change(CandleExchange::COMPOSITE, "AAPL&N"), "AAPL");
        assert_eq!(change(CandlePriceLevel::value_of(0.5).unwrap(), "AAPL"), "AAPL{pl=0.5}");
        assert_eq!(change(CandlePriceLevel::DEFAULT, "AAPL{pl=0.5}"), "AAPL");
        assert_eq!(change(CandleDataType::THO, "AAPL{=d}"), "AAPL{=d,dt=tho}");
        assert_eq!(change(CandleDataType::DEFAULT, "AAPL{dt=tho}"), "AAPL");
        assert_eq!(change(CandlePrice::Last, "AAPL{=d,price=bid}"), "AAPL{=d}");
        assert_eq!(change(CandlePeriod::TICK, "AAPL{=d}"), "AAPL");
        assert_eq!(CandlePeriod::value_of(1.5, CandleType::Hour).to_string(), "1.5h");
        assert_eq!(CandlePeriod::value_of(2.0, CandleType::Hour).to_string(), "2h");
        assert_eq!(CandlePeriod::DAY.to_string(), "d");
    }

    #[test]
    fn values_parse_as_in_java() {
        assert_eq!(CandleType::parse("d").unwrap(), CandleType::Day);
        assert_eq!(CandleType::parse("Days").unwrap(), CandleType::Day);
        assert_eq!(CandleType::parse("min").unwrap(), CandleType::Minute);
        assert_eq!(CandleType::parse("mo").unwrap(), CandleType::Month);
        assert_eq!(CandlePrice::parse("Mark").unwrap(), CandlePrice::Mark);
        assert_eq!(CandleType::parse("Minutes").unwrap(), CandleType::Minute);
        assert!(CandleType::parse("Minutess").is_err());
        assert!(CandleType::parse("Mins").is_err());
        assert_eq!(CandleType::parse("m").unwrap(), CandleType::Minute);
        assert_eq!(CandlePrice::parse("BI").unwrap(), CandlePrice::Bid);
        assert!(CandlePrice::parse("").is_err());
        assert!(CandlePrice::parse("settle").is_err());
        assert_eq!(CandleSession::parse("f").unwrap(), CandleSession::Any);
        assert_eq!(CandleSession::parse("t").unwrap(), CandleSession::Regular);
        assert!(CandleSession::parse("regular").is_err());
        assert_eq!(CandleAlignment::parse("s").unwrap(), CandleAlignment::Session);
        assert_eq!(CandleAlignment::parse("M").unwrap(), CandleAlignment::Midnight);
        assert!(CandleAlignment::parse("session").is_err());
        assert!(CandleType::parse("").is_err());
        assert!(CandlePrice::parse("zzz").is_err());
    }

    #[test]
    fn parse_errors_read_as_in_java() {
        let message = |result: Result<CandlePrice>| match result {
            Err(Error::InvalidArgument(message)) => message,
            other => panic!("{other:?}"),
        };
        assert_eq!(message(CandlePrice::parse("")), "Missing candle price");
        assert_eq!(message(CandlePrice::parse("zzz")), "Unknown candle price: zzz");
        assert!(matches!(
            CandleType::parse("zzz"),
            Err(Error::InvalidArgument(message)) if message == "Unknown candle type: zzz"
        ));
    }

    #[test]
    fn java_case_folding_is_char_wise() {
        // The Kelvin sign lowers to `k`; `ß` has no single upper case and stays itself.
        assert!(java_equals_ignore_case("\u{212a}", "k"));
        assert!(java_equals_ignore_case("Tick", "tICK"));
        assert!(!java_equals_ignore_case("\u{df}", "SS"));
        assert!(!java_equals_ignore_case("tic", "tick"));
    }

    #[test]
    fn java_doubles_parse_as_in_java() {
        assert_eq!(parse_java_double(" 1.5d "), Some(1.5));
        assert_eq!(parse_java_double("1E3"), Some(1000.0));
        assert_eq!(parse_java_double("-Infinity"), Some(f64::NEG_INFINITY));
        assert!(parse_java_double("NaN").is_some_and(f64::is_nan));
        assert_eq!(parse_java_double("inf"), None);
        assert_eq!(parse_java_double("nan"), None);
        assert_eq!(parse_java_double("d"), None);
        assert_eq!(parse_java_double(""), None);
    }
}
