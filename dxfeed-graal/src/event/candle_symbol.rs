//! The symbols of candles and their attributes (`CandleSymbol`, `CandlePeriod`, `CandleType`, `CandlePrice`,
//! `CandleSession`, `CandleAlignment`, `CandlePriceLevel`, `CandleExchange`).

use std::fmt::{self, Display, Formatter};
use std::ptr::{self, NonNull};
use std::str::FromStr;

use dxfeed_graal_sys as sys;

use super::format::JavaDouble;
use crate::error::{Error, JavaException, Result, c_string};
use crate::isolate::{self, call};

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
            text = attribute.into().change(&text);
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
        f.write_str(&self.0)
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
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CandleAttribute {
    /// The exchange code of the trades of the candles, as `&Q` after the base symbol; `'\0'` for the composite of all
    /// exchanges, the default (`CandleExchange`).
    Exchange(char),
    /// The aggregation period, as `=5m` (`CandlePeriod`).
    Period(CandlePeriod),
    /// The price the candles aggregate, as `price=bid` (`CandlePrice`).
    Price(CandlePrice),
    /// The trading session of the candles, as `tho=true` (`CandleSession`).
    Session(CandleSession),
    /// The alignment of the candles, as `a=s` (`CandleAlignment`).
    Alignment(CandleAlignment),
    /// The price level of the candles of the price types, as `pl=0.5`; `NaN` for none, the default
    /// (`CandlePriceLevel`).
    PriceLevel(f64),
}

impl CandleAttribute {
    /// `symbol` with this attribute, as the Java attributes change a symbol (`changeAttributeForSymbol`): a default
    /// value removes the attribute, another value replaces that of the key in place or goes before the first attribute
    /// whose key is greater.
    fn change(self, symbol: &str) -> String {
        let (base, attributes) = split(symbol);
        let (key, value) = match self {
            CandleAttribute::Exchange(code) => {
                let base = without_exchange(base);
                let base = if code == '\0' {
                    base.to_owned()
                } else {
                    format!("{base}&{code}")
                };
                return join(&base, attributes);
            }
            CandleAttribute::Period(period) => ("", (period != CandlePeriod::TICK).then(|| period.to_string())),
            CandleAttribute::Price(price) => ("price", (price != CandlePrice::Last).then(|| price.to_string())),
            CandleAttribute::Session(session) => ("tho", (session != CandleSession::Any).then(|| session.to_string())),
            CandleAttribute::Alignment(alignment) => (
                "a",
                (alignment != CandleAlignment::Midnight).then(|| alignment.to_string()),
            ),
            CandleAttribute::PriceLevel(level) => ("pl", (!level.is_nan()).then(|| format_number(level))),
        };
        let at = attributes
            .iter()
            .position(|(k, _)| *k >= key)
            .unwrap_or(attributes.len());
        let mut attributes: Vec<(&str, &str)> = attributes.into_iter().filter(|(k, _)| *k != key).collect();
        if let Some(value) = &value {
            attributes.insert(at, (key, value));
        }
        join(base, attributes)
    }
}

impl From<CandlePeriod> for CandleAttribute {
    fn from(period: CandlePeriod) -> Self {
        CandleAttribute::Period(period)
    }
}

impl From<CandlePrice> for CandleAttribute {
    fn from(price: CandlePrice) -> Self {
        CandleAttribute::Price(price)
    }
}

impl From<CandleSession> for CandleAttribute {
    fn from(session: CandleSession) -> Self {
        CandleAttribute::Session(session)
    }
}

impl From<CandleAlignment> for CandleAttribute {
    fn from(alignment: CandleAlignment) -> Self {
        CandleAttribute::Alignment(alignment)
    }
}

/// The aggregation period of candles: a number of units of a [`CandleType`], such as 5 minutes (`CandlePeriod`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CandlePeriod {
    value: f64,
    candle_type: CandleType,
}

impl CandlePeriod {
    /// One tick: the default period, each trade is a candle (`CandlePeriod.TICK`).
    pub const TICK: CandlePeriod = CandlePeriod::value_of(1.0, CandleType::Tick);
    /// One day (`CandlePeriod.DAY`).
    pub const DAY: CandlePeriod = CandlePeriod::value_of(1.0, CandleType::Day);

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
        CandlePeriod::TICK
    }
}

impl Display for CandlePeriod {
    /// Prints the period as Java does: the unit alone for one unit, such as `d`, else the number and the unit, such as
    /// `5m`.
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        if self.value == 1.0 {
            write!(f, "{}", self.candle_type)
        } else {
            write!(f, "{}{}", format_number(self.value), self.candle_type)
        }
    }
}

/// A number as Java prints the values of candle attributes: a whole number without a fraction, any other as
/// `Double.toString` prints it.
fn format_number(value: f64) -> String {
    if value.is_finite() && value.fract() == 0.0 && value.abs() < 9.2e18 {
        (value as i64).to_string()
    } else {
        JavaDouble(value).to_string()
    }
}

/// Declares an enum of a candle attribute with the Java names and strings of its values, and its parsing.
macro_rules! candle_enum {
    ($(#[$meta:meta])* $name:ident { $($(#[$vmeta:meta])* $variant:ident = ($java:literal, $string:literal),)* }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name {
            $( $(#[$vmeta])* $variant, )*
        }

        impl $name {
            const VALUES: &'static [($name, &'static str, &'static str)] = &[ $( ($name::$variant, $java, $string), )* ];

            /// The string of the value in a candle symbol, such as `bid` (`toString`).
            pub fn as_str(self) -> &'static str {
                match self {
                    $( $name::$variant => $string, )*
                }
            }

            /// Parses the string of a value, or the start of its Java name in any case, as Java's `parse` does; a
            /// plural such as `Days` is also accepted.
            ///
            /// Returns [`Error::InvalidArgument`] for any other text.
            pub fn parse(text: &str) -> Result<$name> {
                parse_value(text, Self::VALUES, stringify!($name))
            }
        }

        impl Display for $name {
            fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
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

candle_enum!(
    /// The unit of a [`CandlePeriod`] (`CandleType`).
    CandleType {
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
    CandlePrice {
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

candle_enum!(
    /// The trading session of candles (`CandleSession`); [`Any`](Self::Any) is the default.
    CandleSession {
        /// All the trading sessions (`false`).
        Any = ("ANY", "false"),
        /// The regular trading session only (`true`).
        Regular = ("REGULAR", "true"),
    }
);

candle_enum!(
    /// The alignment of candles (`CandleAlignment`); [`Midnight`](Self::Midnight) is the default.
    CandleAlignment {
        /// Aligned to the midnight (`m`).
        Midnight = ("MIDNIGHT", "m"),
        /// Aligned to the start of the trading session (`s`).
        Session = ("SESSION", "s"),
    }
);

/// Finds a value by its string, else by the start of its Java name in any case, as Java's `parse` of candle attributes
/// does.
fn parse_value<T: Copy>(text: &str, values: &[(T, &str, &str)], what: &str) -> Result<T> {
    let found = values.iter().find(|(_, _, string)| *string == text).or_else(|| {
        let n = text.len();
        values.iter().find(|(_, name, _)| {
            let starts = |prefix: &str| name.get(..prefix.len()).is_some_and(|p| p.eq_ignore_ascii_case(prefix));
            !text.is_empty() && (n <= name.len() && starts(text) || text.ends_with('s') && starts(&text[..n - 1]))
        })
    });
    found
        .map(|(value, _, _)| *value)
        .ok_or_else(|| Error::InvalidArgument(format!("unknown {what} {text:?}")))
}

/// The base of `symbol` and its attributes, as `(key, value)` pairs: the period has the empty key.
fn split(symbol: &str) -> (&str, Vec<(&str, &str)>) {
    let Some(open) = symbol.strip_suffix('}').and_then(|s| s.rfind('{')) else {
        return (symbol, Vec::new());
    };
    let attributes = symbol[open + 1..symbol.len() - 1]
        .split(',')
        .filter(|a| !a.is_empty())
        .map(|a| a.split_once('=').unwrap_or((a, "")))
        .collect();
    (&symbol[..open], attributes)
}

/// The symbol of `base` and `attributes`.
fn join(base: &str, attributes: Vec<(&str, &str)>) -> String {
    if attributes.is_empty() {
        return base.to_owned();
    }
    let attributes: Vec<String> = attributes.iter().map(|(k, v)| format!("{k}={v}")).collect();
    format!("{base}{{{}}}", attributes.join(","))
}

/// `base` without its exchange code, the `&Q` at its end.
fn without_exchange(base: &str) -> &str {
    let mut chars = base.char_indices().rev();
    match (chars.next(), chars.next()) {
        (Some(_), Some((amp, '&'))) => &base[..amp],
        _ => base,
    }
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
    let _ = call(|t| unsafe { sys::dxfg_v4_Symbol_release(t, symbol.as_ptr()) });
    normalized.ok_or_else(|| Error::Java(JavaException::from_message("the SDK returned no candle symbol")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attributes_change_the_text() {
        let period = CandleAttribute::Period(CandlePeriod::value_of(5.0, CandleType::Minute));
        assert_eq!(period.change("AAPL"), "AAPL{=5m}");
        assert_eq!(period.change("AAPL{=d,price=bid}"), "AAPL{=5m,price=bid}");
        assert_eq!(period.change("AAPL{price=bid}"), "AAPL{=5m,price=bid}");
        assert_eq!(
            CandleAttribute::Price(CandlePrice::Bid).change("AAPL{=d,tho=true}"),
            "AAPL{=d,price=bid,tho=true}"
        );
        assert_eq!(
            CandleAttribute::Session(CandleSession::Regular).change("AAPL{=d}"),
            "AAPL{=d,tho=true}"
        );
        assert_eq!(CandleAttribute::Exchange('Q').change("AAPL&N{=d}"), "AAPL&Q{=d}");
        assert_eq!(CandleAttribute::Exchange('\0').change("AAPL&N"), "AAPL");
        assert_eq!(CandleAttribute::PriceLevel(0.5).change("AAPL"), "AAPL{pl=0.5}");
        assert_eq!(
            CandleAttribute::Price(CandlePrice::Last).change("AAPL{=d,price=bid}"),
            "AAPL{=d}"
        );
        assert_eq!(CandleAttribute::Period(CandlePeriod::TICK).change("AAPL{=d}"), "AAPL");
        assert_eq!(CandlePeriod::value_of(1.5, CandleType::Hour).to_string(), "1.5h");
        assert_eq!(CandlePeriod::DAY.to_string(), "d");
    }

    #[test]
    fn values_parse_as_in_java() {
        assert_eq!(CandleType::parse("d").unwrap(), CandleType::Day);
        assert_eq!(CandleType::parse("Days").unwrap(), CandleType::Day);
        assert_eq!(CandleType::parse("min").unwrap(), CandleType::Minute);
        assert_eq!(CandleType::parse("mo").unwrap(), CandleType::Month);
        assert_eq!(CandlePrice::parse("Mark").unwrap(), CandlePrice::Mark);
        assert_eq!(CandleSession::parse("regular").unwrap(), CandleSession::Regular);
        assert_eq!(CandleAlignment::parse("s").unwrap(), CandleAlignment::Session);
        assert!(CandleType::parse("").is_err());
        assert!(CandlePrice::parse("zzz").is_err());
    }
}
