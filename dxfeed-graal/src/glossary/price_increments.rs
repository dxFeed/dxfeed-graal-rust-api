//! Price increments (`PriceIncrements`) and rounding modes (`RoundingMode`).

use std::fmt;
use std::hash::{Hash, Hasher};
use std::ptr;
use std::str::FromStr;
use std::sync::Arc;

use dxfeed_graal_sys as sys;

use super::out;
use crate::error::{Error, JavaException, Result, c_string};
use crate::handle::Handle;
use crate::isolate::{Thread, call, out_string};

/// How [`PriceIncrements::round_price_with_mode`] rounds a price, as Java's `java.math.RoundingMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RoundingMode {
    /// Away from zero (`UP`).
    Up,
    /// Towards zero (`DOWN`).
    Down,
    /// Towards positive infinity (`CEILING`).
    Ceiling,
    /// Towards negative infinity (`FLOOR`).
    Floor,
    /// To the nearest value; a tie goes away from zero (`HALF_UP`).
    HalfUp,
    /// To the nearest value; a tie goes towards zero (`HALF_DOWN`).
    HalfDown,
    /// To the nearest value; a tie goes to the even neighbor (`HALF_EVEN`).
    HalfEven,
    /// No rounding: a price that needs it is an error (`UNNECESSARY`).
    Unnecessary,
}

impl RoundingMode {
    /// All modes, in the order of Java.
    pub const ALL: [RoundingMode; 8] = [
        RoundingMode::Up,
        RoundingMode::Down,
        RoundingMode::Ceiling,
        RoundingMode::Floor,
        RoundingMode::HalfUp,
        RoundingMode::HalfDown,
        RoundingMode::HalfEven,
        RoundingMode::Unnecessary,
    ];

    /// The Java name of the mode, such as `HALF_UP` (`name`).
    #[doc(alias = "name")]
    pub const fn get_name(self) -> &'static str {
        match self {
            RoundingMode::Up => "UP",
            RoundingMode::Down => "DOWN",
            RoundingMode::Ceiling => "CEILING",
            RoundingMode::Floor => "FLOOR",
            RoundingMode::HalfUp => "HALF_UP",
            RoundingMode::HalfDown => "HALF_DOWN",
            RoundingMode::HalfEven => "HALF_EVEN",
            RoundingMode::Unnecessary => "UNNECESSARY",
        }
    }

    fn to_sys(self) -> sys::dxfg_v4_rounding_mode_t {
        match self {
            RoundingMode::Up => sys::DXFG_ROUNDING_MODE_UP,
            RoundingMode::Down => sys::DXFG_ROUNDING_MODE_DOWN,
            RoundingMode::Ceiling => sys::DXFG_ROUNDING_MODE_CEILING,
            RoundingMode::Floor => sys::DXFG_ROUNDING_MODE_FLOOR,
            RoundingMode::HalfUp => sys::DXFG_ROUNDING_MODE_HALF_UP,
            RoundingMode::HalfDown => sys::DXFG_ROUNDING_MODE_HALF_DOWN,
            RoundingMode::HalfEven => sys::DXFG_ROUNDING_MODE_HALF_EVEN,
            RoundingMode::Unnecessary => sys::DXFG_ROUNDING_MODE_UNNECESSARY,
        }
    }
}

impl fmt::Display for RoundingMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.get_name())
    }
}

impl FromStr for RoundingMode {
    type Err = Error;

    /// The mode of a Java name, such as `HALF_UP` (`valueOf`); [`Error::InvalidArgument`] for another text.
    fn from_str(name: &str) -> Result<RoundingMode> {
        RoundingMode::ALL
            .into_iter()
            .find(|mode| mode.get_name() == name)
            .ok_or_else(|| Error::InvalidArgument(format!("unknown rounding mode: {name}")))
    }
}

/// The rules of valid prices of an instrument (`PriceIncrements`): price ranges, each with the increment that valid
/// prices in it are multiples of. Prices below zero use the same ranges mirrored.
///
/// The text of the rules lists the ranges as `INCREMENT UPPER_LIMIT` in the ascending order of the limits, separated
/// by `; `; the last range extends to infinity and has no limit. So `0.01 1; 0.05` is 0.01 up to 1 and 0.05 above
/// 1. The rules of [`empty`](Self::empty), for unknown rules, have an empty text and a sole increment of 0.
///
/// A value wraps the Java object, so texts are read and prices are computed as in Java. Values are equal when their
/// texts are, and `Display` writes the text, as in Java. Clones share the Java object.
///
/// ```no_run
/// # fn main() -> dxfeed_graal::Result<()> {
/// use dxfeed_graal::glossary::PriceIncrements;
///
/// let increments = PriceIncrements::value_of("0.01 1; 0.05")?;
/// assert_eq!(increments.get_price_increment_for(2.0)?, 0.05);
/// assert_eq!(increments.round_price(1.234)?, 1.25);
/// assert_eq!(increments.increment_price(0.99, 1)?, 1.0);
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct PriceIncrements {
    handle: Arc<Handle<sys::dxfg_v4_price_increments_t>>,
    text: String,
}

impl PriceIncrements {
    /// The rules of an unknown instrument: an empty text and a sole increment of 0 (`EMPTY`).
    #[doc(alias = "EMPTY")]
    pub fn empty() -> Result<PriceIncrements> {
        // SAFETY: the out-pointer is valid for writes.
        PriceIncrements::create(|t, out| unsafe { sys::dxfg_v4_PriceIncrements_EMPTY(t, out) })
    }

    /// The rules of a text such as `0.01 1; 0.05` (`valueOf(String)`); an empty text or a zero such as `0.00` gives
    /// [`empty`](Self::empty).
    ///
    /// Returns [`Error::Java`] for a text of a wrong format or with invalid values and [`Error::InvalidArgument`] for
    /// a NUL byte.
    #[doc(alias = "valueOf")]
    pub fn value_of(text: &str) -> Result<PriceIncrements> {
        let text = c_string(text, "the price increments")?;
        // SAFETY: the text is NUL-terminated and the out-pointer is valid for writes.
        PriceIncrements::create(|t, out| unsafe { sys::dxfg_v4_PriceIncrements_valueOf(t, text.as_ptr(), out) })
    }

    /// The rules of one increment for all prices (`valueOf(double)`); 0 gives [`empty`](Self::empty).
    ///
    /// Returns [`Error::Java`] for an invalid increment, such as a negative one.
    #[doc(alias = "valueOf")]
    pub fn value_of_increment(increment: f64) -> Result<PriceIncrements> {
        // SAFETY: the out-pointer is valid for writes.
        PriceIncrements::create(|t, out| unsafe { sys::dxfg_v4_PriceIncrements_valueOfIncrement(t, increment, out) })
    }

    /// The rules of increments interleaved with the upper limits of their ranges, as
    /// [`get_price_increments`](Self::get_price_increments) gives them (`valueOf(double[])`).
    ///
    /// Returns [`Error::Java`] for invalid values, such as an even count or limits that do not grow, and
    /// [`Error::InvalidArgument`] for more than `i32::MAX` values.
    #[doc(alias = "valueOf")]
    pub fn value_of_increments(increments: &[f64]) -> Result<PriceIncrements> {
        let size = i32::try_from(increments.len())
            .map_err(|_| Error::InvalidArgument(format!("too many price increments: {}", increments.len())))?;
        let list = sys::dxfg_v4_double_list {
            size,
            elements: increments.as_ptr().cast_mut(),
        };
        // SAFETY: the list points to `size` values that the SDK only reads; the out-pointer is valid for writes.
        PriceIncrements::create(|t, out| unsafe { sys::dxfg_v4_PriceIncrements_valueOfIncrements(t, &list, out) })
    }

    /// Makes the rules with `f` and copies their text.
    fn create(f: impl FnOnce(Thread, *mut *mut sys::dxfg_v4_price_increments_t) -> i32) -> Result<PriceIncrements> {
        let handle = Handle::create(f)?;
        let h = handle.as_ptr();
        // SAFETY: the rules are live while `handle` is, and the out-pointer is valid for writes.
        let text = out_string("PriceIncrements", |t, out| unsafe {
            sys::dxfg_v4_PriceIncrements_getText(t, h, out.cast())
        })?;
        Ok(PriceIncrements {
            handle: Arc::new(handle),
            text,
        })
    }

    fn as_ptr(&self) -> *mut sys::dxfg_v4_price_increments_t {
        self.handle.as_ptr()
    }

    /// The text of the rules, such as `0.01 1; 0.05`; empty for [`empty`](Self::empty) (`getText`).
    #[doc(alias = "getText")]
    pub fn get_text(&self) -> &str {
        &self.text
    }

    /// The numbers of the text: the increments at even positions interleaved with the upper limits of their ranges
    /// (`getPriceIncrements`). The count is odd, since the last range has no limit, and the first increment is there
    /// even for [`empty`](Self::empty), as 0.
    #[doc(alias = "getPriceIncrements")]
    pub fn get_price_increments(&self) -> Result<Vec<f64>> {
        let h = self.as_ptr();
        let mut list = ptr::null_mut();
        // SAFETY: the rules are live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_PriceIncrements_getPriceIncrements(t, h, &mut list) })?;
        if list.is_null() {
            return Err(Error::Java(JavaException::null_result()));
        }
        // SAFETY: the SDK wrote a list of `size` values, NULL `elements` only when empty; it is released once below.
        let values = unsafe {
            let size = usize::try_from((*list).size).unwrap_or(0);
            if size == 0 || (*list).elements.is_null() {
                Vec::new()
            } else {
                std::slice::from_raw_parts((*list).elements, size).to_vec()
            }
        };
        // SAFETY: the list came from the SDK to us and nothing uses it afterwards.
        let released = call(|t| unsafe { sys::dxfg_v4_double_list_release(t, list) });
        crate::error_manager::report_release("PriceIncrements", released);
        Ok(values)
    }

    /// The first increment, of the range next to 0, usually the smallest one; 0 for [`empty`](Self::empty)
    /// (`getPriceIncrement()`).
    #[doc(alias = "getPriceIncrement")]
    pub fn get_price_increment(&self) -> Result<f64> {
        let h = self.as_ptr();
        // SAFETY: the rules are live and the out-pointer is valid for writes.
        out(|t, value| unsafe { sys::dxfg_v4_PriceIncrements_getPriceIncrement(t, h, value) })
    }

    /// The increment for `price` (`getPriceIncrement(double)`): the first one for NaN, the smaller one of the two
    /// ranges for a price at their limit.
    #[doc(alias = "getPriceIncrement")]
    pub fn get_price_increment_for(&self, price: f64) -> Result<f64> {
        let h = self.as_ptr();
        // SAFETY: the rules are live and the out-pointer is valid for writes.
        out(|t, value| unsafe { sys::dxfg_v4_PriceIncrements_getPriceIncrementForPrice(t, h, price, value) })
    }

    /// The increment for `price` when it moves in `direction`, up for a positive one and down for a negative one
    /// (`getPriceIncrement(double, int)`): the first one for NaN; at a limit of two ranges, the increment of the range
    /// in the direction, or the smaller one for direction 0.
    #[doc(alias = "getPriceIncrement")]
    pub fn get_price_increment_in_direction(&self, price: f64, direction: i32) -> Result<f64> {
        let h = self.as_ptr();
        out(|t, value| {
            // SAFETY: the rules are live and the out-pointer is valid for writes.
            unsafe { sys::dxfg_v4_PriceIncrements_getPriceIncrementForPriceAndDirection(t, h, price, direction, value) }
        })
    }

    /// The precision of the first range, next to 0, usually the largest one; 0 for [`empty`](Self::empty)
    /// (`getPricePrecision()`).
    #[doc(alias = "getPricePrecision")]
    pub fn get_price_precision(&self) -> Result<i32> {
        let h = self.as_ptr();
        // SAFETY: the rules are live and the out-pointer is valid for writes.
        out(|t, value| unsafe { sys::dxfg_v4_PriceIncrements_getPricePrecision(t, h, value) })
    }

    /// The number of decimal digits after the point that the valid prices of the range of `price` need, from 0 to 18
    /// (`getPricePrecision(double)`): the first one for NaN, that of the lower range for a price at a limit.
    #[doc(alias = "getPricePrecision")]
    pub fn get_price_precision_for(&self, price: f64) -> Result<i32> {
        let h = self.as_ptr();
        // SAFETY: the rules are live and the out-pointer is valid for writes.
        out(|t, value| unsafe { sys::dxfg_v4_PriceIncrements_getPricePrecisionForPrice(t, h, price, value) })
    }

    /// `price` rounded to the nearest valid price (`roundPrice(double)`); NaN stays NaN, and a price whose increment
    /// is 0 stays as it is.
    #[doc(alias = "roundPrice")]
    pub fn round_price(&self, price: f64) -> Result<f64> {
        let h = self.as_ptr();
        // SAFETY: the rules are live and the out-pointer is valid for writes.
        out(|t, value| unsafe { sys::dxfg_v4_PriceIncrements_roundPrice(t, h, price, value) })
    }

    /// `price` rounded to the nearest valid price in `direction`, up for a positive one, down for a negative one and
    /// to the nearest for 0 (`roundPrice(double, int)`); NaN stays NaN, and a price whose increment is 0 stays as it
    /// is.
    #[doc(alias = "roundPrice")]
    pub fn round_price_in_direction(&self, price: f64, direction: i32) -> Result<f64> {
        let h = self.as_ptr();
        out(|t, value| {
            // SAFETY: the rules are live and the out-pointer is valid for writes.
            unsafe { sys::dxfg_v4_PriceIncrements_roundPriceWithDirection(t, h, price, direction, value) }
        })
    }

    /// `price` rounded to a valid price by `mode` (`roundPrice(double, RoundingMode)`); NaN stays NaN, and a price
    /// whose increment is 0 stays as it is.
    ///
    /// Returns [`Error::Java`] for [`RoundingMode::Unnecessary`] and a price that is not valid.
    #[doc(alias = "roundPrice")]
    pub fn round_price_with_mode(&self, price: f64, mode: RoundingMode) -> Result<f64> {
        let (h, mode) = (self.as_ptr(), mode.to_sys());
        out(|t, value| {
            // SAFETY: the rules are live and the out-pointer is valid for writes.
            unsafe { sys::dxfg_v4_PriceIncrements_roundPriceWithRoundingMode(t, h, price, mode, value) }
        })
    }

    /// `price` moved by its increment in `direction`, up for a positive one and down for a negative one, then rounded
    /// to the nearest valid price (`incrementPrice(double, int)`); NaN stays NaN, and a price whose increment is 0
    /// stays as it is.
    ///
    /// Returns [`Error::Java`] for direction 0.
    #[doc(alias = "incrementPrice")]
    pub fn increment_price(&self, price: f64, direction: i32) -> Result<f64> {
        let h = self.as_ptr();
        // SAFETY: the rules are live and the out-pointer is valid for writes.
        out(|t, value| unsafe { sys::dxfg_v4_PriceIncrements_incrementPrice(t, h, price, direction, value) })
    }

    /// `price` moved in `direction` by the larger of `step` and its increment, then rounded to the nearest valid price
    /// (`incrementPrice(double, int, double)`); `step` is positive in both directions. NaN stays NaN, and a price
    /// stays as it is when both `step` and its increment are 0.
    ///
    /// Returns [`Error::Java`] for direction 0 or an invalid step.
    #[doc(alias = "incrementPrice")]
    pub fn increment_price_with_step(&self, price: f64, direction: i32, step: f64) -> Result<f64> {
        let h = self.as_ptr();
        out(|t, value| {
            // SAFETY: the rules are live and the out-pointer is valid for writes.
            unsafe { sys::dxfg_v4_PriceIncrements_incrementPriceWithStep(t, h, price, direction, step, value) }
        })
    }
}

impl PartialEq for PriceIncrements {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text
    }
}

impl Eq for PriceIncrements {}

impl Hash for PriceIncrements {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.text.hash(state);
    }
}

impl fmt::Debug for PriceIncrements {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("PriceIncrements").field(&self.text).finish()
    }
}

impl fmt::Display for PriceIncrements {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

impl FromStr for PriceIncrements {
    type Err = Error;

    /// As [`value_of`](PriceIncrements::value_of).
    fn from_str(text: &str) -> Result<PriceIncrements> {
        PriceIncrements::value_of(text)
    }
}
