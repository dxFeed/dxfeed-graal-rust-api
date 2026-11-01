//! The instrument profile and its conversion to and from the SDK structure.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::ffi::{CString, c_char};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ptr;
use std::str::FromStr;

use dxfeed_graal_sys as sys;

use crate::error::{Error, JavaException, Result, c_string};
use crate::handle::Handle;
use crate::isolate::{self, call};

/// The reference data of an instrument: its symbol, type, codes, trading terms and so on (`InstrumentProfile`).
///
/// As in Java, a text field that is not set is empty and a number that is not set is zero. Dates are day ids: the
/// number of days since January 1, 1970. A profile whose type is `REMOVED` tells that the instrument was removed
/// (see [`is_removed`](Self::is_removed)).
///
/// [`Display`](fmt::Display) gives the type and the symbol, like `toString` in Java: `STOCK AAPL`. Profiles are equal
/// and hash as in Java's `equals` and `hashCode`: a NaN number equals NaN, `0.0` differs from `-0.0`, and a custom
/// field with an empty value counts as missing.
#[derive(Debug, Clone, Default)]
pub struct InstrumentProfile {
    /// The type of the instrument, such as `STOCK`, `FUTURE`, `OPTION` or `REMOVED` (`getType`).
    #[doc(alias = "type")]
    pub instrument_type: String,
    /// The identifier of the instrument in dxFeed, such as `AAPL` (`getSymbol`).
    pub symbol: String,
    /// The description of the instrument (`getDescription`).
    pub description: String,
    /// The identifier of the instrument in the national language (`getLocalSymbol`).
    pub local_symbol: String,
    /// The description of the instrument in the national language (`getLocalDescription`).
    pub local_description: String,
    /// The country of origin, an ISO 3166-1 alpha-2 code (`getCountry`).
    pub country: String,
    /// The official place of listing, a Market Identifier Code (`getOPOL`).
    #[doc(alias = "getOPOL")]
    pub opol: String,
    /// The exchange-specific data, such as the Reuters code (`getExchangeData`).
    pub exchange_data: String,
    /// The exchanges where the instrument is traded, Market Identifier Codes separated by `;` (`getExchanges`).
    pub exchanges: String,
    /// The currency of quotation, pricing and trading, an ISO 4217 code (`getCurrency`).
    pub currency: String,
    /// The base currency of a currency pair, an ISO 4217 code (`getBaseCurrency`).
    pub base_currency: String,
    /// The Classification of Financial Instruments code (`getCFI`).
    #[doc(alias = "getCFI")]
    pub cfi: String,
    /// The International Securities Identifying Number (`getISIN`).
    #[doc(alias = "getISIN")]
    pub isin: String,
    /// The Stock Exchange Daily Official List code (`getSEDOL`).
    #[doc(alias = "getSEDOL")]
    pub sedol: String,
    /// The Committee on Uniform Security Identification Procedures code (`getCUSIP`).
    #[doc(alias = "getCUSIP")]
    pub cusip: String,
    /// The Industry Classification Benchmark number (`getICB`).
    #[doc(alias = "getICB")]
    pub icb: i32,
    /// The Standard Industrial Classification number (`getSIC`).
    #[doc(alias = "getSIC")]
    pub sic: i32,
    /// The market value multiplier (`getMultiplier`).
    pub multiplier: f64,
    /// The product of futures and options on futures, such as `/YG` (`getProduct`).
    pub product: String,
    /// The primary underlying symbol of options (`getUnderlying`).
    pub underlying: String,
    /// The shares per contract of options (`getSPC`).
    #[doc(alias = "getSPC")]
    pub spc: f64,
    /// The additional underlyings of options, such as `SE 50; FIS 53; US$ 45.46` (`getAdditionalUnderlyings`).
    pub additional_underlyings: String,
    /// The maturity month-year, such as `201406` (`getMMY`).
    #[doc(alias = "getMMY")]
    pub mmy: String,
    /// The day id of the expiration (`getExpiration`).
    pub expiration: i32,
    /// The day id of the last trading day (`getLastTrade`).
    pub last_trade: i32,
    /// The strike price of options (`getStrike`).
    pub strike: f64,
    /// The type of an option, such as `STAN` or `LEAP` (`getOptionType`).
    pub option_type: String,
    /// The expiration cycle style, such as `Weeklys` or `Quarterlys` (`getExpirationStyle`).
    pub expiration_style: String,
    /// The settlement price determination style, such as `Open` or `Close` (`getSettlementStyle`).
    pub settlement_style: String,
    /// The minimum allowed price increments with the price ranges they apply to (`getPriceIncrements`).
    pub price_increments: String,
    /// The trading hours specification (`getTradingHours`).
    pub trading_hours: String,
    /// The fields that are not standard, by name. Fields with empty values are not kept by the SDK
    /// (`getField` and `setField` with other names).
    pub custom_fields: BTreeMap<String, String>,
}

impl PartialEq for InstrumentProfile {
    fn eq(&self, other: &Self) -> bool {
        let InstrumentProfile {
            instrument_type,
            symbol,
            description,
            local_symbol,
            local_description,
            country,
            opol,
            exchange_data,
            exchanges,
            currency,
            base_currency,
            cfi,
            isin,
            sedol,
            cusip,
            icb,
            sic,
            multiplier,
            product,
            underlying,
            spc,
            additional_underlyings,
            mmy,
            expiration,
            last_trade,
            strike,
            option_type,
            expiration_style,
            settlement_style,
            price_increments,
            trading_hours,
            custom_fields,
        } = self;
        let InstrumentProfile {
            instrument_type: other_instrument_type,
            symbol: other_symbol,
            description: other_description,
            local_symbol: other_local_symbol,
            local_description: other_local_description,
            country: other_country,
            opol: other_opol,
            exchange_data: other_exchange_data,
            exchanges: other_exchanges,
            currency: other_currency,
            base_currency: other_base_currency,
            cfi: other_cfi,
            isin: other_isin,
            sedol: other_sedol,
            cusip: other_cusip,
            icb: other_icb,
            sic: other_sic,
            multiplier: other_multiplier,
            product: other_product,
            underlying: other_underlying,
            spc: other_spc,
            additional_underlyings: other_additional_underlyings,
            mmy: other_mmy,
            expiration: other_expiration,
            last_trade: other_last_trade,
            strike: other_strike,
            option_type: other_option_type,
            expiration_style: other_expiration_style,
            settlement_style: other_settlement_style,
            price_increments: other_price_increments,
            trading_hours: other_trading_hours,
            custom_fields: other_custom_fields,
        } = other;
        instrument_type == other_instrument_type
            && symbol == other_symbol
            && description == other_description
            && local_symbol == other_local_symbol
            && local_description == other_local_description
            && country == other_country
            && opol == other_opol
            && exchange_data == other_exchange_data
            && exchanges == other_exchanges
            && currency == other_currency
            && base_currency == other_base_currency
            && cfi == other_cfi
            && isin == other_isin
            && sedol == other_sedol
            && cusip == other_cusip
            && product == other_product
            && underlying == other_underlying
            && additional_underlyings == other_additional_underlyings
            && mmy == other_mmy
            && option_type == other_option_type
            && expiration_style == other_expiration_style
            && settlement_style == other_settlement_style
            && price_increments == other_price_increments
            && trading_hours == other_trading_hours
            && icb == other_icb
            && sic == other_sic
            && expiration == other_expiration
            && last_trade == other_last_trade
            && java_bits(*multiplier) == java_bits(*other_multiplier)
            && java_bits(*spc) == java_bits(*other_spc)
            && java_bits(*strike) == java_bits(*other_strike)
            && non_empty(custom_fields).eq(non_empty(other_custom_fields))
    }
}

impl Eq for InstrumentProfile {}

impl Hash for InstrumentProfile {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let InstrumentProfile {
            instrument_type,
            symbol,
            description,
            local_symbol,
            local_description,
            country,
            opol,
            exchange_data,
            exchanges,
            currency,
            base_currency,
            cfi,
            isin,
            sedol,
            cusip,
            icb,
            sic,
            multiplier,
            product,
            underlying,
            spc,
            additional_underlyings,
            mmy,
            expiration,
            last_trade,
            strike,
            option_type,
            expiration_style,
            settlement_style,
            price_increments,
            trading_hours,
            custom_fields,
        } = self;
        instrument_type.hash(state);
        symbol.hash(state);
        description.hash(state);
        local_symbol.hash(state);
        local_description.hash(state);
        country.hash(state);
        opol.hash(state);
        exchange_data.hash(state);
        exchanges.hash(state);
        currency.hash(state);
        base_currency.hash(state);
        cfi.hash(state);
        isin.hash(state);
        sedol.hash(state);
        cusip.hash(state);
        product.hash(state);
        underlying.hash(state);
        additional_underlyings.hash(state);
        mmy.hash(state);
        option_type.hash(state);
        expiration_style.hash(state);
        settlement_style.hash(state);
        price_increments.hash(state);
        trading_hours.hash(state);
        icb.hash(state);
        sic.hash(state);
        expiration.hash(state);
        last_trade.hash(state);
        java_bits(*multiplier).hash(state);
        java_bits(*spc).hash(state);
        java_bits(*strike).hash(state);
        non_empty(custom_fields).for_each(|field| field.hash(state));
    }
}

/// The bits of a number as Java's `Double.doubleToLongBits` gives them: every NaN has the same bits.
fn java_bits(value: f64) -> u64 {
    if value.is_nan() {
        f64::NAN.to_bits()
    } else {
        value.to_bits()
    }
}

/// The custom fields that Java keeps: those with a value.
fn non_empty(fields: &BTreeMap<String, String>) -> impl Iterator<Item = (&String, &String)> {
    fields.iter().filter(|(_, value)| !value.is_empty())
}

impl fmt::Display for InstrumentProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.instrument_type, self.symbol)
    }
}

/// The table of the standard fields: their variants of [`InstrumentProfileField`], their names in the Instrument
/// Profile Format, the fields of the profile and of the SDK structure, and their kind. It gives the access by name and
/// the conversion to and from the SDK.
macro_rules! standard_fields {
    ($($variant:ident $name:literal => $field:ident / $sys:ident: $kind:ident),* $(,)?) => {
        /// A standard field of an instrument profile (`InstrumentProfileField`).
        ///
        /// [`Display`](fmt::Display) and [`FromStr`] use the name of the field in the Instrument Profile Format, the
        /// name of the Java constant: `LOCAL_SYMBOL`. The other fields of a profile are custom fields.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum InstrumentProfileField {
            $(
                #[doc = concat!("`", $name, "`: [`InstrumentProfile::", stringify!($field), "`].")]
                $variant,
            )*
        }

        impl InstrumentProfileField {
            /// Every field, in the order of the Instrument Profile Format and of Java's
            /// `InstrumentProfileField.values()`.
            #[doc(alias = "values")]
            pub const ALL: &'static [InstrumentProfileField] = &[$(InstrumentProfileField::$variant),*];

            /// The field with the name `name`, or `None` for a custom field (`find`).
            pub fn find(name: &str) -> Option<InstrumentProfileField> {
                match name {
                    $($name => Some(InstrumentProfileField::$variant),)*
                    _ => None,
                }
            }

            /// The name of the field in the Instrument Profile Format, such as `LOCAL_SYMBOL` (`name`).
            #[doc(alias = "name")]
            pub const fn get_name(self) -> &'static str {
                match self {
                    $(InstrumentProfileField::$variant => $name,)*
                }
            }

            /// Whether the field is a number or a date, which [`get_numeric_field`](Self::get_numeric_field) reads
            /// (`isNumericField`).
            #[doc(alias = "isNumericField")]
            pub const fn is_numeric_field(self) -> bool {
                match self {
                    $(InstrumentProfileField::$variant => standard_fields!(@numeric $kind),)*
                }
            }

            /// The value of the field of `profile` as text, formatted as in the Instrument Profile Format; empty if it
            /// is not set (`getField`).
            #[doc(alias = "getField")]
            pub fn get_field(self, profile: &InstrumentProfile) -> Result<String> {
                match self {
                    $(InstrumentProfileField::$variant => standard_fields!(@get $kind, profile.$field),)*
                }
            }

            /// Sets the field of `profile` from text, parsed as in the Instrument Profile Format; an empty value clears
            /// it (`setField`).
            ///
            /// Returns an error if the value of a numeric or date field cannot be parsed.
            #[doc(alias = "setField")]
            pub fn set_field(self, profile: &mut InstrumentProfile, value: &str) -> Result<()> {
                match self {
                    $(InstrumentProfileField::$variant => standard_fields!(@set $kind, profile.$field, value),)*
                }
            }

            /// The value of a numeric field of `profile`; a date is its day id (`getNumericField`).
            ///
            /// Returns [`Error::InvalidArgument`] for a text field.
            #[doc(alias = "getNumericField")]
            pub fn get_numeric_field(self, profile: &InstrumentProfile) -> Result<f64> {
                match self {
                    $(InstrumentProfileField::$variant => standard_fields!(@get_numeric $kind, self, profile.$field),)*
                }
            }

            /// Sets a numeric field of `profile`; an integer or a date field takes the value cast to an integer, as in
            /// Java (`setNumericField`).
            ///
            /// Returns [`Error::InvalidArgument`] for a text field.
            #[doc(alias = "setNumericField")]
            pub fn set_numeric_field(self, profile: &mut InstrumentProfile, value: f64) -> Result<()> {
                match self {
                    $(InstrumentProfileField::$variant =>
                        standard_fields!(@set_numeric $kind, self, profile.$field, value),)*
                }
            }
        }

        impl InstrumentProfile {
            /// The names of the standard fields, in the order of the Instrument Profile Format.
            pub const STANDARD_FIELDS: &[&str] = &[$($name),*];

            /// Copies a profile from an SDK structure.
            ///
            /// # Safety
            ///
            /// `data` must point to a live structure.
            pub(crate) unsafe fn from_sys(data: *const sys::dxfg_v4_instrument_profile_data_t) -> Result<Self> {
                // SAFETY: the caller guarantees the structure is live; its strings are NUL-terminated or NULL.
                let data = unsafe { &*data };
                Ok(InstrumentProfile {
                    $($field: standard_fields!(@from $kind, data.$sys),)*
                    // SAFETY: the handle belongs to the live structure.
                    custom_fields: unsafe { custom_fields(data.instrument_profile_custom_fields) }?,
                })
            }

            /// Builds the SDK structure of this profile, which borrows its strings.
            pub(crate) fn to_sys(&self) -> Result<SysProfile> {
                let mut strings = Vec::new();
                let custom = new_custom_fields(&self.custom_fields)?;
                let data = sys::dxfg_v4_instrument_profile_data_t {
                    $($sys: standard_fields!(@to $kind, self.$field, strings, $name),)*
                    instrument_profile_custom_fields: custom.as_ref().map_or(ptr::null_mut(), Handle::as_ptr),
                    instrument_profile_custom_fields_hash: 0,
                };
                Ok(SysProfile { data, _strings: strings, _custom: custom })
            }
        }
    };
    (@numeric text) => { false };
    (@numeric $kind:ident) => { true };
    (@get text, $v:expr) => { Ok($v.clone()) };
    (@get int, $v:expr) => { InstrumentProfileField::format_number(f64::from($v)) };
    (@get number, $v:expr) => { InstrumentProfileField::format_number($v) };
    (@get date, $v:expr) => { InstrumentProfileField::format_date($v) };
    (@set text, $v:expr, $value:ident) => {{ $v = $value.to_owned(); Ok(()) }};
    // Java casts the parsed number to an int as well.
    (@set int, $v:expr, $value:ident) => { InstrumentProfileField::parse_number($value).map(|n| $v = n as i32) };
    (@set number, $v:expr, $value:ident) => { InstrumentProfileField::parse_number($value).map(|n| $v = n) };
    (@set date, $v:expr, $value:ident) => { InstrumentProfileField::parse_date($value).map(|d| $v = d) };
    (@get_numeric text, $f:ident, $v:expr) => { Err(textual($f)) };
    (@get_numeric number, $f:ident, $v:expr) => { Ok($v) };
    (@get_numeric $kind:ident, $f:ident, $v:expr) => { Ok(f64::from($v)) };
    (@set_numeric text, $f:ident, $v:expr, $value:ident) => { Err(textual($f)) };
    (@set_numeric number, $f:ident, $v:expr, $value:ident) => {{ $v = $value; Ok(()) }};
    (@set_numeric $kind:ident, $f:ident, $v:expr, $value:ident) => {{ $v = $value as i32; Ok(()) }};
    // SAFETY: the strings of a live structure are NUL-terminated or NULL.
    (@from text, $v:expr) => { unsafe { isolate::string($v) }.into_owned() };
    (@from $kind:ident, $v:expr) => { $v };
    (@to text, $v:expr, $strings:ident, $name:literal) => { text(&$v, $name, &mut $strings)? };
    (@to $kind:ident, $v:expr, $strings:ident, $name:literal) => { $v };
}

standard_fields! {
    Type "TYPE" => instrument_type / type_: text,
    Symbol "SYMBOL" => symbol / symbol: text,
    Description "DESCRIPTION" => description / description: text,
    LocalSymbol "LOCAL_SYMBOL" => local_symbol / local_symbol: text,
    LocalDescription "LOCAL_DESCRIPTION" => local_description / local_description: text,
    Country "COUNTRY" => country / country: text,
    Opol "OPOL" => opol / opol: text,
    ExchangeData "EXCHANGE_DATA" => exchange_data / exchange_data: text,
    Exchanges "EXCHANGES" => exchanges / exchanges: text,
    Currency "CURRENCY" => currency / currency: text,
    BaseCurrency "BASE_CURRENCY" => base_currency / base_currency: text,
    Cfi "CFI" => cfi / cfi: text,
    Isin "ISIN" => isin / isin: text,
    Sedol "SEDOL" => sedol / sedol: text,
    Cusip "CUSIP" => cusip / cusip: text,
    Icb "ICB" => icb / icb: int,
    Sic "SIC" => sic / sic: int,
    Multiplier "MULTIPLIER" => multiplier / multiplier: number,
    Product "PRODUCT" => product / product: text,
    Underlying "UNDERLYING" => underlying / underlying: text,
    Spc "SPC" => spc / spc: number,
    AdditionalUnderlyings "ADDITIONAL_UNDERLYINGS" => additional_underlyings / additional_underlyings: text,
    Mmy "MMY" => mmy / mmy: text,
    Expiration "EXPIRATION" => expiration / expiration: date,
    LastTrade "LAST_TRADE" => last_trade / last_trade: date,
    Strike "STRIKE" => strike / strike: number,
    OptionType "OPTION_TYPE" => option_type / option_type: text,
    ExpirationStyle "EXPIRATION_STYLE" => expiration_style / expiration_style: text,
    SettlementStyle "SETTLEMENT_STYLE" => settlement_style / settlement_style: text,
    PriceIncrements "PRICE_INCREMENTS" => price_increments / price_increments: text,
    TradingHours "TRADING_HOURS" => trading_hours / trading_hours: text,
}

/// The error of a numeric access to a text field, as Java's `IllegalArgumentException`.
fn textual(field: InstrumentProfileField) -> Error {
    Error::InvalidArgument(format!("textual field {field}"))
}

impl InstrumentProfileField {
    /// Formats a number as the Instrument Profile Format writes it: `100`, `0.5`, and zero as an empty string
    /// (`formatNumber`).
    #[doc(alias = "formatNumber")]
    pub fn format_number(number: f64) -> Result<String> {
        let mut text = ptr::null_mut();
        // SAFETY: the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_InstrumentProfileField_formatNumber(t, number, &mut text) })?;
        // SAFETY: the SDK wrote a NUL-terminated string or NULL.
        Ok(unsafe { take_string(text) })
    }

    /// Parses a number as the Instrument Profile Format reads it; an empty string is zero (`parseNumber`).
    ///
    /// Returns [`Error::Java`] for text that is not a number and [`Error::InvalidArgument`] for a NUL byte.
    #[doc(alias = "parseNumber")]
    pub fn parse_number(text: &str) -> Result<f64> {
        let text = c_string(text, "the number")?;
        let mut number = 0.0;
        // SAFETY: the string is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_InstrumentProfileField_parseNumber(t, text.as_ptr(), &mut number) })?;
        Ok(number)
    }

    /// Formats a day id, the number of days since January 1, 1970, as `yyyy-MM-dd`, and zero as an empty string
    /// (`formatDate`).
    #[doc(alias = "formatDate")]
    pub fn format_date(day_id: i32) -> Result<String> {
        let mut text = ptr::null_mut();
        // SAFETY: the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_InstrumentProfileField_formatDate(t, day_id, &mut text) })?;
        // SAFETY: the SDK wrote a NUL-terminated string or NULL.
        Ok(unsafe { take_string(text) })
    }

    /// Parses a `yyyy-MM-dd` date into a day id; an empty string is zero (`parseDate`).
    ///
    /// Returns [`Error::Java`] for text that is not a date and [`Error::InvalidArgument`] for a NUL byte.
    #[doc(alias = "parseDate")]
    pub fn parse_date(text: &str) -> Result<i32> {
        let text = c_string(text, "the date")?;
        let mut date = 0;
        // SAFETY: the string is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_InstrumentProfileField_parseDate(t, text.as_ptr(), &mut date) })?;
        Ok(date)
    }
}

impl fmt::Display for InstrumentProfileField {
    /// Writes the name of the field, such as `LOCAL_SYMBOL`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.get_name())
    }
}

impl FromStr for InstrumentProfileField {
    type Err = Error;

    /// Parses the name of a field, as Java's `valueOf(String)` does; the case matters.
    ///
    /// Returns [`Error::InvalidArgument`] for any other string.
    fn from_str(name: &str) -> Result<Self> {
        Self::find(name).ok_or_else(|| Error::InvalidArgument(format!("unknown InstrumentProfileField {name:?}")))
    }
}

/// The table of the types of instruments: their variants of [`InstrumentProfileType`] and their names.
macro_rules! profile_types {
    ($($(#[$doc:meta])* $variant:ident $name:literal),* $(,)?) => {
        /// A standard type of an instrument (`InstrumentProfileType`); the type of a profile is the text of its
        /// [`instrument_type`](InstrumentProfile::instrument_type), which may be another one.
        ///
        /// [`Display`](fmt::Display) and [`FromStr`] use the name of the Java constant: `MUTUAL_FUND`. The order of
        /// the values is the order of Java's constants.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum InstrumentProfileType {
            $($(#[$doc])* $variant,)*
        }

        impl InstrumentProfileType {
            /// Every type, in the order of Java's `InstrumentProfileType.values()`.
            #[doc(alias = "values")]
            pub const ALL: &'static [InstrumentProfileType] = &[$(InstrumentProfileType::$variant),*];

            /// The type with the name `name`, or `None` for another name (`find`).
            pub fn find(name: &str) -> Option<InstrumentProfileType> {
                match name {
                    $($name => Some(InstrumentProfileType::$variant),)*
                    _ => None,
                }
            }

            /// The name of the type, such as `MUTUAL_FUND` (`name`).
            #[doc(alias = "name")]
            pub const fn get_name(self) -> &'static str {
                match self {
                    $(InstrumentProfileType::$variant => $name,)*
                }
            }
        }
    };
}

profile_types! {
    /// Currencies (`CURRENCY`).
    Currency "CURRENCY",
    /// Currency pairs (`FOREX`).
    Forex "FOREX",
    /// Bonds (`BOND`).
    Bond "BOND",
    /// Indices (`INDEX`).
    Index "INDEX",
    /// Stocks (`STOCK`).
    Stock "STOCK",
    /// Exchange-traded funds (`ETF`).
    Etf "ETF",
    /// Mutual funds (`MUTUAL_FUND`).
    MutualFund "MUTUAL_FUND",
    /// Money market funds (`MONEY_MARKET_FUND`).
    MoneyMarketFund "MONEY_MARKET_FUND",
    /// The products of futures and options on futures (`PRODUCT`).
    Product "PRODUCT",
    /// Futures (`FUTURE`).
    Future "FUTURE",
    /// Options (`OPTION`).
    Option "OPTION",
    /// Warrants (`WARRANT`).
    Warrant "WARRANT",
    /// Certificates (`CERTIFICATE`).
    Certificate "CERTIFICATE",
    /// Contracts for difference (`CFD`).
    Cfd "CFD",
    /// Spreads (`SPREAD`).
    Spread "SPREAD",
    /// Other instruments (`OTHER`).
    Other "OTHER",
    /// A removed instrument (`REMOVED`): see [`InstrumentProfile::is_removed`].
    Removed "REMOVED",
}

impl InstrumentProfileType {
    /// Compares two types by name as Java's `compareTypes` does: the standard types in the order of [`ALL`](Self::ALL),
    /// then the other names in alphabetical order. The order serves files and displays, not business logic.
    #[doc(alias = "compareTypes")]
    pub fn compare_types(type1: &str, type2: &str) -> Ordering {
        match (Self::find(type1), Self::find(type2)) {
            (Some(t1), Some(t2)) => t1.cmp(&t2),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => java_compare(type1, type2),
        }
    }
}

impl fmt::Display for InstrumentProfileType {
    /// Writes the name of the type, such as `MUTUAL_FUND`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.get_name())
    }
}

impl FromStr for InstrumentProfileType {
    type Err = Error;

    /// Parses the name of a type, as Java's `valueOf(String)` does; the case matters.
    ///
    /// Returns [`Error::InvalidArgument`] for any other string.
    fn from_str(name: &str) -> Result<Self> {
        Self::find(name).ok_or_else(|| Error::InvalidArgument(format!("unknown InstrumentProfileType {name:?}")))
    }
}

/// Whether a custom value is taken for a date, `yyyy-MM-dd`: 10 UTF-16 units with `-` at 4 and 7, as in Java.
fn looks_like_date(value: &str) -> bool {
    let units: Vec<u16> = value.encode_utf16().collect();
    units.len() == 10 && units[4] == u16::from(b'-') && units[7] == u16::from(b'-')
}

/// Compares strings as Java's `String.compareTo` does: by UTF-16 code units.
fn java_compare(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// Compares numbers as Java's `a > b ? 1 : a < b ? -1 : 0` does: NaN is equal to everything.
fn java_compare_numbers<T: PartialOrd>(a: T, b: T) -> Ordering {
    a.partial_cmp(&b).unwrap_or(Ordering::Equal)
}

impl InstrumentProfile {
    /// The type of a profile that tells that its instrument was removed (`InstrumentProfileType.REMOVED`).
    pub const REMOVED: &str = "REMOVED";

    /// Creates an empty profile.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the type of the profile is [`REMOVED`](Self::REMOVED).
    pub fn is_removed(&self) -> bool {
        self.instrument_type == Self::REMOVED
    }

    /// The value of a field by its name in the Instrument Profile Format, standard or custom, as text; empty if it
    /// is not set (`getField`).
    ///
    /// Numbers and dates are formatted as in the format: `100`, `0.5`, `2026-12-18`, and zero as an empty string.
    #[doc(alias = "getField")]
    pub fn get_field(&self, name: &str) -> Result<String> {
        match InstrumentProfileField::find(name) {
            Some(field) => field.get_field(self),
            None => Ok(self.custom_fields.get(name).cloned().unwrap_or_default()),
        }
    }

    /// Sets a field by its name in the Instrument Profile Format, standard or custom, from text (`setField`).
    ///
    /// Numbers and dates are parsed as in the format; an empty value clears the field. Returns an error if the
    /// value of a numeric or date field cannot be parsed.
    #[doc(alias = "setField")]
    pub fn set_field(&mut self, name: &str, value: &str) -> Result<()> {
        match InstrumentProfileField::find(name) {
            Some(field) => field.set_field(self, value),
            None => {
                self.set_custom_field(name, value);
                Ok(())
            }
        }
    }

    /// The value of a numeric field by its name, standard or custom; a date is its day id (`getNumericField`).
    ///
    /// A custom field that is not set is zero, and one of the form `yyyy-MM-dd` is read as a date, as in Java.
    /// Returns [`Error::InvalidArgument`] for a standard text field and an error if a custom value is not a number.
    #[doc(alias = "getNumericField")]
    pub fn get_numeric_field(&self, name: &str) -> Result<f64> {
        if let Some(field) = InstrumentProfileField::find(name) {
            return field.get_numeric_field(self);
        }
        match self.custom_fields.get(name).map(String::as_str) {
            None | Some("") => Ok(0.0),
            Some(value) if looks_like_date(value) => InstrumentProfileField::parse_date(value).map(f64::from),
            Some(value) => InstrumentProfileField::parse_number(value),
        }
    }

    /// Sets a numeric field by its name, standard or custom (`setNumericField`). A custom field gets the number
    /// formatted as in the format, so zero clears it.
    ///
    /// Returns [`Error::InvalidArgument`] for a standard text field.
    #[doc(alias = "setNumericField")]
    pub fn set_numeric_field(&mut self, name: &str, value: f64) -> Result<()> {
        match InstrumentProfileField::find(name) {
            Some(field) => field.set_numeric_field(self, value),
            None => {
                let value = InstrumentProfileField::format_number(value)?;
                self.set_custom_field(name, &value);
                Ok(())
            }
        }
    }

    /// The day id of a date field by its name, standard or custom; zero if it is not set (`getDateField`).
    ///
    /// A standard numeric field gives its value cast to an integer, as in Java. Returns [`Error::InvalidArgument`]
    /// for a standard text field and an error if a custom value is not a date.
    #[doc(alias = "getDateField")]
    pub fn get_date_field(&self, name: &str) -> Result<i32> {
        if let Some(field) = InstrumentProfileField::find(name) {
            return field.get_numeric_field(self).map(|value| value as i32);
        }
        match self.custom_fields.get(name).map(String::as_str) {
            None | Some("") => Ok(0),
            Some(value) => InstrumentProfileField::parse_date(value),
        }
    }

    /// Sets a date field by its name, standard or custom, to a day id (`setDateField`). A custom field gets the date
    /// formatted as `yyyy-MM-dd`, so zero clears it.
    ///
    /// Returns [`Error::InvalidArgument`] for a standard text field.
    #[doc(alias = "setDateField")]
    pub fn set_date_field(&mut self, name: &str, day_id: i32) -> Result<()> {
        match InstrumentProfileField::find(name) {
            Some(field) => field.set_numeric_field(self, f64::from(day_id)),
            None => {
                let value = InstrumentProfileField::format_date(day_id)?;
                self.set_custom_field(name, &value);
                Ok(())
            }
        }
    }

    /// Compares two profiles as Java's `compareTo` does: by the type ([`InstrumentProfileType::compare_types`]), the
    /// product, the underlying, the last trade day, the strike and the symbol. Profiles that differ in other fields
    /// compare as equal, so it is not [`Ord`]; sort with `profiles.sort_by(InstrumentProfile::compare_to)`.
    #[doc(alias = "compareTo")]
    pub fn compare_to(&self, other: &Self) -> Ordering {
        InstrumentProfileType::compare_types(&self.instrument_type, &other.instrument_type)
            .then_with(|| java_compare(&self.product, &other.product))
            .then_with(|| java_compare(&self.underlying, &other.underlying))
            .then_with(|| java_compare_numbers(self.last_trade, other.last_trade))
            .then_with(|| java_compare_numbers(self.strike, other.strike))
            .then_with(|| java_compare(&self.symbol, &other.symbol))
    }

    fn set_custom_field(&mut self, name: &str, value: &str) {
        if value.is_empty() {
            self.custom_fields.remove(name);
        } else {
            self.custom_fields.insert(name.to_owned(), value.to_owned());
        }
    }
}

/// The SDK structure of a profile with the strings and the custom fields it points to.
pub(crate) struct SysProfile {
    pub(crate) data: sys::dxfg_v4_instrument_profile_data_t,
    _strings: Vec<CString>,
    _custom: Option<Handle<sys::dxfg_v4_instrument_profile_custom_fields_t>>,
}

/// Keeps a copy of `value` in `strings` and returns a pointer to it; the SDK only reads it.
fn text(value: &str, name: &str, strings: &mut Vec<CString>) -> Result<*mut c_char> {
    let copy = c_string(value, name)?;
    // The buffer of a CString does not move with it.
    let pointer = copy.as_ptr().cast_mut();
    strings.push(copy);
    Ok(pointer)
}

/// Creates the SDK custom fields of a profile, none if there are no non-empty ones.
fn new_custom_fields(
    fields: &BTreeMap<String, String>,
) -> Result<Option<Handle<sys::dxfg_v4_instrument_profile_custom_fields_t>>> {
    let mut strings = Vec::new();
    for (name, value) in fields.iter().filter(|(_, value)| !value.is_empty()) {
        strings.push(c_string(name, "a custom field name")?);
        strings.push(c_string(value, "a custom field value")?);
    }
    if strings.is_empty() {
        return Ok(None);
    }
    let mut pointers: Vec<*const c_char> = strings.iter().map(|s| s.as_ptr()).collect();
    let list = sys::dxfg_v4_string_list {
        size: pointers.len() as i32,
        elements: pointers.as_mut_ptr(),
    };
    // SAFETY: the list and its strings live until the call returns; the out-pointer is valid for writes.
    Handle::create(|t, out| unsafe { sys::dxfg_v4_InstrumentProfileCustomFields_newWithNamesAndValues(t, &list, out) })
        .map(Some)
}

/// Copies the non-empty custom fields of an SDK handle, which may be NULL.
///
/// # Safety
///
/// `handle` must be NULL or live.
unsafe fn custom_fields(
    handle: *mut sys::dxfg_v4_instrument_profile_custom_fields_t,
) -> Result<BTreeMap<String, String>> {
    let mut fields = BTreeMap::new();
    if handle.is_null() {
        return Ok(fields);
    }
    let mut names = ptr::null_mut();
    // SAFETY: the handle is live and the out-pointer is valid for writes.
    call(|t| unsafe { sys::dxfg_v4_InstrumentProfileCustomFields_getNonEmptyFieldNames(t, handle, &mut names) })?;
    if names.is_null() {
        return Err(Error::Java(JavaException::null_result()));
    }
    let mut read = || -> Result<()> {
        // SAFETY: the SDK filled the list, which lives until it is released below.
        let list = unsafe { &*names };
        let elements = match list.size {
            0 => &[][..],
            // SAFETY: the list has `size` elements.
            size => unsafe { std::slice::from_raw_parts(list.elements, size as usize) },
        };
        for &name in elements {
            let mut value = ptr::null_mut();
            // SAFETY: the handle and the name are live; the out-pointer is valid for writes.
            call(|t| unsafe { sys::dxfg_v4_InstrumentProfileCustomFields_getField(t, handle, name, &mut value) })?;
            // SAFETY: the SDK wrote a NUL-terminated string or NULL.
            let text = unsafe { take_string(value) };
            // SAFETY: the name is a NUL-terminated string of the list.
            fields.insert(unsafe { isolate::string(name) }.into_owned(), text);
        }
        Ok(())
    };
    let result = read();
    // SAFETY: the list came from the SDK and is released once.
    crate::error_manager::report_release(
        "InstrumentProfile",
        call(|t| unsafe { sys::dxfg_v4_string_list_release(t, names) }),
    );
    result.map(|()| fields)
}

/// Copies a string that the SDK returned and releases it; NULL gives an empty string.
///
/// # Safety
///
/// `s` must be NULL or a NUL-terminated string that the SDK returned to the caller.
pub(crate) unsafe fn take_string(s: *const c_char) -> String {
    // SAFETY: the caller guarantees the string.
    let copy = unsafe { isolate::string(s) }.into_owned();
    if !s.is_null() {
        // SAFETY: the string is the caller's and is released once.
        crate::error_manager::report_release(
            "InstrumentProfile",
            call(|t| unsafe { sys::dxfg_v4_String_release(t, s) }),
        );
    }
    copy
}
