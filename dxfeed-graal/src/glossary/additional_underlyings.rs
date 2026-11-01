//! Additional underlyings of an option (`AdditionalUnderlyings`).

use std::collections::BTreeMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ptr;
use std::str::FromStr;
use std::sync::Arc;

use dxfeed_graal_sys as sys;

use super::out;
use crate::error::{Error, JavaException, Result, c_string};
use crate::handle::Handle;
use crate::isolate::{self, Thread, call, out_string};

/// The underlyings of an option besides its main one, with the shares per contract (SPC) of each
/// (`AdditionalUnderlyings`). An option that settles with additional cash names the cash as a symbol, with the amount
/// as its SPC.
///
/// The text lists the underlyings as `UNDERLYING SPC`, separated by `; `, such as `SE 50; FIS 53`; the texts that
/// Java makes list them in the alphabetical order of the symbols. The value of [`empty`](Self::empty) has an empty
/// text and no underlyings.
///
/// A value wraps the Java object, so texts are read as in Java. Values are equal when their texts are, and `Display`
/// writes the text, as in Java. Clones share the Java object.
///
/// ```no_run
/// # fn main() -> dxfeed_graal::Result<()> {
/// use dxfeed_graal::glossary::AdditionalUnderlyings;
///
/// let underlyings = AdditionalUnderlyings::value_of("SE 50; FIS 53")?;
/// assert_eq!(underlyings.get_spc("SE")?, 50.0);
/// assert_eq!(underlyings.get_spc("IBM")?, 0.0);
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct AdditionalUnderlyings {
    handle: Arc<Handle<sys::dxfg_v4_additional_underlyings_t>>,
    text: String,
}

impl AdditionalUnderlyings {
    /// No additional underlyings: an empty text (`EMPTY`).
    #[doc(alias = "EMPTY")]
    pub fn empty() -> Result<AdditionalUnderlyings> {
        // SAFETY: the out-pointer is valid for writes.
        AdditionalUnderlyings::create(|t, out| unsafe { sys::dxfg_v4_AdditionalUnderlyings_EMPTY(t, out) })
    }

    /// The underlyings of a text such as `SE 50; FIS 53` (`valueOf(String)`); an empty text gives
    /// [`empty`](Self::empty). The text is kept as it is.
    ///
    /// Returns [`Error::Java`] for a text of a wrong format or with invalid values and [`Error::InvalidArgument`] for
    /// a NUL byte.
    #[doc(alias = "valueOf")]
    pub fn value_of(text: &str) -> Result<AdditionalUnderlyings> {
        let text = c_string(text, "the additional underlyings")?;
        AdditionalUnderlyings::create(|t, out| {
            // SAFETY: the text is NUL-terminated and the out-pointer is valid for writes.
            unsafe { sys::dxfg_v4_AdditionalUnderlyings_valueOf(t, text.as_ptr(), out) }
        })
    }

    /// The underlyings of pairs of a symbol and its SPC (`valueOf(Map)`); the text lists them in the alphabetical
    /// order of the symbols. A symbol that comes twice keeps its last SPC.
    ///
    /// Returns [`Error::Java`] for an invalid pair, such as an empty symbol, one with a space or `;`, or a SPC that is
    /// not finite, and [`Error::InvalidArgument`] for a symbol with a NUL byte or more than `i32::MAX` pairs.
    #[doc(alias = "valueOf")]
    pub fn value_of_map<S: AsRef<str>>(map: impl IntoIterator<Item = (S, f64)>) -> Result<AdditionalUnderlyings> {
        let mut pairs = BTreeMap::new();
        for (symbol, spc) in map {
            pairs.insert(c_string(symbol.as_ref(), "the symbol")?, spc);
        }
        let mut entries: Vec<_> = pairs
            .iter()
            .map(|(symbol, &spc)| sys::dxfg_v4_string_to_double_map_entry_t {
                key: symbol.as_ptr(),
                value: spc,
            })
            .collect();
        let size = i32::try_from(entries.len())
            .map_err(|_| Error::InvalidArgument(format!("too many additional underlyings: {}", entries.len())))?;
        let list = sys::dxfg_v4_string_to_double_map_entry_list {
            size,
            elements: entries.as_mut_ptr(),
        };
        AdditionalUnderlyings::create(|t, out| {
            // SAFETY: the list and its keys are live and the SDK only reads them; the out-pointer is valid for writes.
            unsafe { sys::dxfg_v4_AdditionalUnderlyings_valueOfMap(t, &list, out) }
        })
    }

    /// The SPC of `symbol` in a text, or 0 if the text does not have it (`getSPC(String, String)`), without making
    /// a value. As in Java, the format of the text is not checked: a part that cannot be read is skipped.
    ///
    /// Returns [`Error::InvalidArgument`] for a NUL byte.
    #[doc(alias = "getSPC")]
    pub fn get_spc_from_text(text: &str, symbol: &str) -> Result<f64> {
        let text = c_string(text, "the additional underlyings")?;
        let symbol = c_string(symbol, "the symbol")?;
        out(|t, spc| {
            // SAFETY: the strings are NUL-terminated and the out-pointer is valid for writes.
            unsafe { sys::dxfg_v4_AdditionalUnderlyings_getSPCFromText(t, text.as_ptr(), symbol.as_ptr(), spc) }
        })
    }

    /// Makes the underlyings with `f` and copies their text.
    fn create(
        f: impl FnOnce(Thread, *mut *mut sys::dxfg_v4_additional_underlyings_t) -> i32,
    ) -> Result<AdditionalUnderlyings> {
        let handle = Handle::create(f)?;
        let h = handle.as_ptr();
        // SAFETY: the underlyings are live while `handle` is, and the out-pointer is valid for writes.
        let text = out_string("AdditionalUnderlyings", |t, out| unsafe {
            sys::dxfg_v4_AdditionalUnderlyings_getText(t, h, out.cast())
        })?;
        Ok(AdditionalUnderlyings {
            handle: Arc::new(handle),
            text,
        })
    }

    fn as_ptr(&self) -> *mut sys::dxfg_v4_additional_underlyings_t {
        self.handle.as_ptr()
    }

    /// The text, such as `SE 50; FIS 53`; empty for [`empty`](Self::empty) (`getText`).
    #[doc(alias = "getText")]
    pub fn get_text(&self) -> &str {
        &self.text
    }

    /// The SPC of every underlying symbol (`getMap`).
    #[doc(alias = "getMap")]
    pub fn get_map(&self) -> Result<BTreeMap<String, f64>> {
        let h = self.as_ptr();
        let mut list = ptr::null_mut();
        // SAFETY: the underlyings are live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_AdditionalUnderlyings_getMap(t, h, &mut list) })?;
        if list.is_null() {
            return Err(Error::Java(JavaException::null_result()));
        }
        // SAFETY: the SDK wrote a list of `size` entries with NUL-terminated keys, NULL `elements` only when empty; it
        // is released once below.
        let map = unsafe {
            let size = usize::try_from((*list).size).unwrap_or(0);
            if size == 0 || (*list).elements.is_null() {
                BTreeMap::new()
            } else {
                std::slice::from_raw_parts((*list).elements, size)
                    .iter()
                    .map(|entry| (isolate::string(entry.key).into_owned(), entry.value))
                    .collect()
            }
        };
        // SAFETY: the list came from the SDK to us and nothing uses it afterwards.
        let released = call(|t| unsafe { sys::dxfg_v4_string_to_double_map_entry_list_release(t, list) });
        crate::error_manager::report_release("AdditionalUnderlyings", released);
        Ok(map)
    }

    /// The SPC of `symbol`, or 0 if it is not an underlying (`getSPC(String)`); [`Error::InvalidArgument`] for a NUL
    /// byte.
    #[doc(alias = "getSPC")]
    pub fn get_spc(&self, symbol: &str) -> Result<f64> {
        let h = self.as_ptr();
        let symbol = c_string(symbol, "the symbol")?;
        out(|t, spc| {
            // SAFETY: the underlyings are live, the symbol is NUL-terminated and the out-pointer is valid for writes.
            unsafe { sys::dxfg_v4_AdditionalUnderlyings_getSPC(t, h, symbol.as_ptr(), spc) }
        })
    }
}

impl PartialEq for AdditionalUnderlyings {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text
    }
}

impl Eq for AdditionalUnderlyings {}

impl Hash for AdditionalUnderlyings {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.text.hash(state);
    }
}

impl fmt::Debug for AdditionalUnderlyings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("AdditionalUnderlyings").field(&self.text).finish()
    }
}

impl fmt::Display for AdditionalUnderlyings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

impl FromStr for AdditionalUnderlyings {
    type Err = Error;

    /// As [`value_of`](AdditionalUnderlyings::value_of).
    fn from_str(text: &str) -> Result<AdditionalUnderlyings> {
        AdditionalUnderlyings::value_of(text)
    }
}
