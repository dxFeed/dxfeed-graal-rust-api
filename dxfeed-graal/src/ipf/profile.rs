//! The instrument profile and its conversion to and from the SDK structure.

use std::collections::BTreeMap;
use std::ffi::{CString, c_char};
use std::fmt;
use std::ptr;

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
/// [`Display`](fmt::Display) gives the type and the symbol, like `toString` in Java: `STOCK AAPL`.
#[derive(Debug, Clone, Default, PartialEq)]
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

impl fmt::Display for InstrumentProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.instrument_type, self.symbol)
    }
}

/// The table of the standard fields: their names in the Instrument Profile Format, the fields of the profile and of
/// the SDK structure, and their kind. It gives the access by name and the conversion to and from the SDK.
macro_rules! standard_fields {
    ($($name:literal => $field:ident / $sys:ident: $kind:ident),* $(,)?) => {
        impl InstrumentProfile {
            /// The names of the standard fields, in the order of the Instrument Profile Format.
            pub const STANDARD_FIELDS: &[&str] = &[$($name),*];

            fn standard_field(&self, name: &str) -> Option<Result<String>> {
                match name {
                    $($name => Some(standard_fields!(@get $kind, self.$field)),)*
                    _ => None,
                }
            }

            fn set_standard_field(&mut self, name: &str, value: &str) -> Option<Result<()>> {
                match name {
                    $($name => Some(standard_fields!(@set $kind, self.$field, value)),)*
                    _ => None,
                }
            }

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
    (@get text, $v:expr) => { Ok($v.clone()) };
    (@get int, $v:expr) => { format_number(f64::from($v)) };
    (@get number, $v:expr) => { format_number($v) };
    (@get date, $v:expr) => { format_date($v) };
    (@set text, $v:expr, $value:ident) => {{ $v = $value.to_owned(); Ok(()) }};
    // Java casts the parsed number to an int as well.
    (@set int, $v:expr, $value:ident) => { parse_number($value).map(|n| $v = n as i32) };
    (@set number, $v:expr, $value:ident) => { parse_number($value).map(|n| $v = n) };
    (@set date, $v:expr, $value:ident) => { parse_date($value).map(|d| $v = d) };
    // SAFETY: the strings of a live structure are NUL-terminated or NULL.
    (@from text, $v:expr) => { unsafe { isolate::string($v) }.into_owned() };
    (@from $kind:ident, $v:expr) => { $v };
    (@to text, $v:expr, $strings:ident, $name:literal) => { text(&$v, $name, &mut $strings)? };
    (@to $kind:ident, $v:expr, $strings:ident, $name:literal) => { $v };
}

standard_fields! {
    "TYPE" => instrument_type / type_: text,
    "SYMBOL" => symbol / symbol: text,
    "DESCRIPTION" => description / description: text,
    "LOCAL_SYMBOL" => local_symbol / local_symbol: text,
    "LOCAL_DESCRIPTION" => local_description / local_description: text,
    "COUNTRY" => country / country: text,
    "OPOL" => opol / opol: text,
    "EXCHANGE_DATA" => exchange_data / exchange_data: text,
    "EXCHANGES" => exchanges / exchanges: text,
    "CURRENCY" => currency / currency: text,
    "BASE_CURRENCY" => base_currency / base_currency: text,
    "CFI" => cfi / cfi: text,
    "ISIN" => isin / isin: text,
    "SEDOL" => sedol / sedol: text,
    "CUSIP" => cusip / cusip: text,
    "ICB" => icb / icb: int,
    "SIC" => sic / sic: int,
    "MULTIPLIER" => multiplier / multiplier: number,
    "PRODUCT" => product / product: text,
    "UNDERLYING" => underlying / underlying: text,
    "SPC" => spc / spc: number,
    "ADDITIONAL_UNDERLYINGS" => additional_underlyings / additional_underlyings: text,
    "MMY" => mmy / mmy: text,
    "EXPIRATION" => expiration / expiration: date,
    "LAST_TRADE" => last_trade / last_trade: date,
    "STRIKE" => strike / strike: number,
    "OPTION_TYPE" => option_type / option_type: text,
    "EXPIRATION_STYLE" => expiration_style / expiration_style: text,
    "SETTLEMENT_STYLE" => settlement_style / settlement_style: text,
    "PRICE_INCREMENTS" => price_increments / price_increments: text,
    "TRADING_HOURS" => trading_hours / trading_hours: text,
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
        self.standard_field(name)
            .unwrap_or_else(|| Ok(self.custom_fields.get(name).cloned().unwrap_or_default()))
    }

    /// Sets a field by its name in the Instrument Profile Format, standard or custom, from text (`setField`).
    ///
    /// Numbers and dates are parsed as in the format; an empty value clears the field. Returns an error if the
    /// value of a numeric or date field cannot be parsed.
    #[doc(alias = "setField")]
    pub fn set_field(&mut self, name: &str, value: &str) -> Result<()> {
        if let Some(set) = self.set_standard_field(name, value) {
            return set;
        }
        if value.is_empty() {
            self.custom_fields.remove(name);
        } else {
            self.custom_fields.insert(name.to_owned(), value.to_owned());
        }
        Ok(())
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
    let _ = call(|t| unsafe { sys::dxfg_v4_string_list_release(t, names) });
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
        let _ = call(|t| unsafe { sys::dxfg_v4_String_release(t, s) });
    }
    copy
}

fn format_number(number: f64) -> Result<String> {
    let mut text = ptr::null_mut();
    // SAFETY: the out-pointer is valid for writes.
    call(|t| unsafe { sys::dxfg_v4_InstrumentProfileField_formatNumber(t, number, &mut text) })?;
    // SAFETY: the SDK wrote a NUL-terminated string or NULL.
    Ok(unsafe { take_string(text) })
}

fn format_date(date: i32) -> Result<String> {
    let mut text = ptr::null_mut();
    // SAFETY: the out-pointer is valid for writes.
    call(|t| unsafe { sys::dxfg_v4_InstrumentProfileField_formatDate(t, date, &mut text) })?;
    // SAFETY: the SDK wrote a NUL-terminated string or NULL.
    Ok(unsafe { take_string(text) })
}

fn parse_number(text: &str) -> Result<f64> {
    let text = c_string(text, "the number")?;
    let mut number = 0.0;
    // SAFETY: the string is live and the out-pointer is valid for writes.
    call(|t| unsafe { sys::dxfg_v4_InstrumentProfileField_parseNumber(t, text.as_ptr(), &mut number) })?;
    Ok(number)
}

fn parse_date(text: &str) -> Result<i32> {
    let text = c_string(text, "the date")?;
    let mut date = 0;
    // SAFETY: the string is live and the out-pointer is valid for writes.
    call(|t| unsafe { sys::dxfg_v4_InstrumentProfileField_parseDate(t, text.as_ptr(), &mut date) })?;
    Ok(date)
}
