//! CFI codes (`CFI`) and their attributes and values (`CFI.Attribute`, `CFI.Value`).

use std::fmt;
use std::hash::{Hash, Hasher};
use std::ptr;
use std::str::FromStr;
use std::sync::Arc;

use dxfeed_graal_sys as sys;

use super::out;
use crate::error::{Error, JavaException, Result, c_string};
use crate::error_manager::report_release;
use crate::handle::Handle;
use crate::isolate::{Thread, call, out_string};

/// A code of the Classification of Financial Instruments, ISO 10962 (`CFI`): six letters, such as `OCASPS` for an
/// option, where each letter has a meaning in the context of the letters before it.
///
/// A code shorter than six letters is filled up with `X`, which stands for an unknown attribute; the code of
/// [`empty`](Self::empty) is `XXXXXX`. [`decipher`](Self::decipher) and [`describe`](Self::describe) explain the
/// letters with the tables of Java.
///
/// Codes are equal when their letters are, and `Display` writes the code, as in Java. Clones share the Java object.
///
/// ```no_run
/// # fn main() -> dxfeed_graal::Result<()> {
/// use dxfeed_graal::glossary::Cfi;
///
/// let cfi = Cfi::value_of("OCASPS")?;
/// assert!(cfi.is_option());
/// assert_eq!(cfi.decipher()?[1].get_name(), "Call options");
/// # Ok(())
/// # }
/// ```
#[doc(alias = "CFI")]
#[derive(Clone)]
pub struct Cfi {
    handle: Arc<Handle<sys::dxfg_v4_cfi_t>>,
    code: String,
    int_code: i32,
}

impl Cfi {
    /// The code of an unknown instrument, `XXXXXX` (`EMPTY`).
    #[doc(alias = "EMPTY")]
    pub fn empty() -> Result<Cfi> {
        // SAFETY: the out-pointer is valid for writes.
        Cfi::create(|t, out| unsafe { sys::dxfg_v4_CFI_EMPTY(t, out) })
    }

    /// The CFI of a code of up to six letters `A` to `Z`, filled up with `X` (`valueOf(String)`); an empty code
    /// gives [`empty`](Self::empty).
    ///
    /// Returns [`Error::Java`] for an invalid code, such as one that is too long or has another character, and
    /// [`Error::InvalidArgument`] for a NUL byte.
    #[doc(alias = "valueOf")]
    pub fn value_of(code: &str) -> Result<Cfi> {
        let code = c_string(code, "the CFI code")?;
        // SAFETY: the code is NUL-terminated and the out-pointer is valid for writes.
        Cfi::create(|t, out| unsafe { sys::dxfg_v4_CFI_valueOf(t, code.as_ptr(), out) })
    }

    /// The CFI of the integer form of a code, as [`get_int_code`](Self::get_int_code) gives it (`valueOf(int)`);
    /// 0 gives [`empty`](Self::empty).
    ///
    /// Returns [`Error::Java`] for an invalid integer.
    #[doc(alias = "valueOf")]
    pub fn value_of_int_code(int_code: i32) -> Result<Cfi> {
        // SAFETY: the out-pointer is valid for writes.
        Cfi::create(|t, out| unsafe { sys::dxfg_v4_CFI_valueOfIntCode(t, int_code, out) })
    }

    /// Makes the CFI with `f` and copies its code.
    fn create(f: impl FnOnce(Thread, *mut *mut sys::dxfg_v4_cfi_t) -> i32) -> Result<Cfi> {
        let handle = Handle::create(f)?;
        let h = handle.as_ptr();
        // SAFETY: the CFI is live while `handle` is, and the out-pointer is valid for writes.
        let code = out_string("CFI", |t, out| unsafe { sys::dxfg_v4_CFI_getCode(t, h, out.cast()) })?;
        // SAFETY: as above.
        let int_code = out(|t, value| unsafe { sys::dxfg_v4_CFI_getIntCode(t, h, value) })?;
        Ok(Cfi {
            handle: Arc::new(handle),
            code,
            int_code,
        })
    }

    /// The code, always of six letters (`getCode`).
    #[doc(alias = "getCode")]
    pub fn get_code(&self) -> &str {
        &self.code
    }

    /// The integer form of the code, five bits a letter (`getIntCode`).
    #[doc(alias = "getIntCode")]
    pub fn get_int_code(&self) -> i32 {
        self.int_code
    }

    /// The letter of the category of the instrument, the first one of the code (`getCategory`).
    #[doc(alias = "getCategory")]
    pub fn get_category(&self) -> char {
        self.letter(0)
    }

    /// The letter of the group of the instrument, the second one of the code (`getGroup`).
    #[doc(alias = "getGroup")]
    pub fn get_group(&self) -> char {
        self.letter(1)
    }

    fn letter(&self, position: usize) -> char {
        char::from(self.code.as_bytes()[position])
    }

    /// Whether the instrument is an equity, category `E` (`isEquity`).
    #[doc(alias = "isEquity")]
    pub fn is_equity(&self) -> bool {
        self.get_category() == 'E'
    }

    /// Whether the instrument is a debt instrument, category `D` (`isDebtInstrument`).
    #[doc(alias = "isDebtInstrument")]
    pub fn is_debt_instrument(&self) -> bool {
        self.get_category() == 'D'
    }

    /// Whether the instrument is an entitlement (a right), category `R` (`isEntitlement`).
    #[doc(alias = "isEntitlement")]
    pub fn is_entitlement(&self) -> bool {
        self.get_category() == 'R'
    }

    /// Whether the instrument is an option, category `O` (`isOption`).
    #[doc(alias = "isOption")]
    pub fn is_option(&self) -> bool {
        self.get_category() == 'O'
    }

    /// Whether the instrument is a future, category `F` (`isFuture`).
    #[doc(alias = "isFuture")]
    pub fn is_future(&self) -> bool {
        self.get_category() == 'F'
    }

    /// Whether the instrument is another (miscellaneous) one, category `M` (`isOther`).
    #[doc(alias = "isOther")]
    pub fn is_other(&self) -> bool {
        self.get_category() == 'M'
    }

    /// The meanings of the six letters of the code, one value each (`decipher`). A letter that does not apply, is
    /// unknown or is not recognized has a value that says so.
    pub fn decipher(&self) -> Result<Vec<CfiValue>> {
        let h = self.handle.as_ptr();
        let mut list = ptr::null_mut();
        // SAFETY: the CFI is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_CFI_decipher(t, h, &mut list) })?;
        // SAFETY: the SDK wrote a list of value handles that is ours.
        unsafe { values(list) }
    }

    /// The names of the values of the six letters, separated by `; ` (`describe`), such as
    /// `Options; Call options; American; Stock-Equities; Physical; Standardized` for `OCASPS`.
    pub fn describe(&self) -> Result<String> {
        let h = self.handle.as_ptr();
        // SAFETY: the CFI is live and the out-pointer is valid for writes.
        out_string("CFI", |t, out| unsafe { sys::dxfg_v4_CFI_describe(t, h, out.cast()) })
    }
}

impl PartialEq for Cfi {
    fn eq(&self, other: &Self) -> bool {
        self.int_code == other.int_code
    }
}

impl Eq for Cfi {}

impl Hash for Cfi {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.int_code.hash(state);
    }
}

impl fmt::Debug for Cfi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Cfi").field(&self.code).finish()
    }
}

impl fmt::Display for Cfi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.code)
    }
}

impl FromStr for Cfi {
    type Err = Error;

    /// As [`value_of`](Cfi::value_of).
    fn from_str(code: &str) -> Result<Cfi> {
        Cfi::value_of(code)
    }
}

/// The meaning of one letter of a [`Cfi`] code in the ISO 10962 standard (`CFI.Value`), a value of a
/// [`CfiAttribute`].
///
/// `Display` writes `code = name`, as Java's `toString`. Clones share the Java object.
#[doc(alias = "CFI.Value")]
#[derive(Clone)]
pub struct CfiValue {
    handle: Arc<Handle<sys::dxfg_v4_cfi_value_t>>,
    code: char,
    name: String,
    description: String,
}

impl CfiValue {
    /// Takes the handle of a value and copies its letter, name and description.
    fn new(handle: Handle<sys::dxfg_v4_cfi_value_t>) -> Result<CfiValue> {
        let h = handle.as_ptr();
        // SAFETY: the value is live while `handle` is, and the out-pointers are valid for writes.
        let code: i16 = out(|t, code| unsafe { sys::dxfg_v4_CFI_Value_getCode(t, h, code) })?;
        // SAFETY: as above.
        let name = out_string("CFI.Value", |t, out| unsafe {
            sys::dxfg_v4_CFI_Value_getName(t, h, out.cast())
        })?;
        let description = out_string("CFI.Value", |t, out| {
            // SAFETY: as above.
            unsafe { sys::dxfg_v4_CFI_Value_getDescription(t, h, out.cast()) }
        })?;
        Ok(CfiValue {
            handle: Arc::new(handle),
            code: java_char(code),
            name,
            description,
        })
    }

    /// The attribute of the letter of this value (`getAttribute`); the value of an unknown or not recognized letter
    /// also has the attribute of its position. `None` only when Java has no attribute for the value.
    #[doc(alias = "getAttribute")]
    pub fn get_attribute(&self) -> Result<Option<CfiAttribute>> {
        let h = self.handle.as_ptr();
        // SAFETY: the value is live and the out-pointer is valid for writes.
        let handle = Handle::create_optional(|t, out| unsafe { sys::dxfg_v4_CFI_Value_getAttribute(t, h, out) })?;
        handle.map(CfiAttribute::new).transpose()
    }

    /// The letter of the value (`getCode`), such as `C`; for an unknown or not recognized letter, the letter itself
    /// with the name `Unknown` or `Unrecognized`.
    #[doc(alias = "getCode")]
    pub fn get_code(&self) -> char {
        self.code
    }

    /// The short name of the value (`getName`), such as `Call options`.
    #[doc(alias = "getName")]
    pub fn get_name(&self) -> &str {
        &self.name
    }

    /// The description of the value (`getDescription`); it may be empty.
    #[doc(alias = "getDescription")]
    pub fn get_description(&self) -> &str {
        &self.description
    }
}

impl fmt::Debug for CfiValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CfiValue")
            .field("code", &self.code)
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for CfiValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} = {}", self.code, self.name)
    }
}

/// An attribute of a letter of a [`Cfi`] code in the ISO 10962 standard, with all its values (`CFI.Attribute`).
///
/// `Display` writes the name, as Java's `toString`. Clones share the Java object.
#[doc(alias = "CFI.Attribute")]
#[derive(Clone)]
pub struct CfiAttribute {
    handle: Arc<Handle<sys::dxfg_v4_cfi_attribute_t>>,
    name: String,
    description: String,
}

impl CfiAttribute {
    /// Takes the handle of an attribute and copies its name and description.
    fn new(handle: Handle<sys::dxfg_v4_cfi_attribute_t>) -> Result<CfiAttribute> {
        let h = handle.as_ptr();
        let name = out_string("CFI.Attribute", |t, out| {
            // SAFETY: the attribute is live while `handle` is, and the out-pointer is valid for writes.
            unsafe { sys::dxfg_v4_CFI_Attribute_getName(t, h, out.cast()) }
        })?;
        let description = out_string("CFI.Attribute", |t, out| {
            // SAFETY: as above.
            unsafe { sys::dxfg_v4_CFI_Attribute_getDescription(t, h, out.cast()) }
        })?;
        Ok(CfiAttribute {
            handle: Arc::new(handle),
            name,
            description,
        })
    }

    /// The short name of the attribute (`getName`), such as `Type of scheme`.
    #[doc(alias = "getName")]
    pub fn get_name(&self) -> &str {
        &self.name
    }

    /// The description of the attribute (`getDescription`); it may be empty.
    #[doc(alias = "getDescription")]
    pub fn get_description(&self) -> &str {
        &self.description
    }

    /// The values of the attribute (`getValues`).
    #[doc(alias = "getValues")]
    pub fn get_values(&self) -> Result<Vec<CfiValue>> {
        let h = self.handle.as_ptr();
        let mut list = ptr::null_mut();
        // SAFETY: the attribute is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_CFI_Attribute_getValues(t, h, &mut list) })?;
        // SAFETY: the SDK wrote a list of value handles that is ours.
        unsafe { values(list) }
    }
}

impl fmt::Debug for CfiAttribute {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CfiAttribute")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for CfiAttribute {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)
    }
}

/// The character of a Java `char`; a lone surrogate gives `U+FFFD`.
fn java_char(code: i16) -> char {
    char::from_u32(u32::from(code as u16)).unwrap_or(char::REPLACEMENT_CHARACTER)
}

/// Copies the values of an SDK list and releases the list with its handles.
///
/// # Safety
///
/// `list` must be NULL or a live list of value handles that the caller owns and releases nowhere else.
unsafe fn values(list: *mut sys::dxfg_v4_cfi_value_list) -> Result<Vec<CfiValue>> {
    if list.is_null() {
        return Err(Error::Java(JavaException::null_result()));
    }
    // SAFETY: the list is live; it has `size` handles, NULL `elements` only when empty.
    let handles = unsafe {
        let size = usize::try_from((*list).size).unwrap_or(0);
        if size == 0 || (*list).elements.is_null() {
            &[][..]
        } else {
            std::slice::from_raw_parts((*list).elements, size)
        }
    };
    // The release of the list releases its handles: each value keeps a clone of its handle.
    let values = handles
        .iter()
        // SAFETY: the handles are live until the list is released below.
        .map(|&handle| unsafe { Handle::clone_raw(handle) }.and_then(CfiValue::new))
        .collect();
    // SAFETY: the list is ours and nothing uses it afterwards.
    let released = call(|t| unsafe { sys::dxfg_v4_cfi_value_list_release(t, list) });
    report_release("CFI", released);
    values
}
