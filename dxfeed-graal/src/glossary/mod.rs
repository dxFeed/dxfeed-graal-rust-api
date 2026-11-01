//! The glossary of instrument attributes (`com.dxfeed.glossary`): the text values of an
//! [`InstrumentProfile`](crate::ipf::InstrumentProfile), such as its
//! [`price_increments`](crate::ipf::InstrumentProfile::price_increments) and
//! [`additional_underlyings`](crate::ipf::InstrumentProfile::additional_underlyings), as typed values, and the
//! [`Cfi`] code of its [`cfi`](crate::ipf::InstrumentProfile::cfi) with the meanings of its letters.
//!
//! The types wrap the Java objects of the SDK, so they read texts and compute prices exactly as Java does.
//!
//! ```no_run
//! # fn main() -> dxfeed_graal::Result<()> {
//! use dxfeed_graal::glossary::{AdditionalUnderlyings, Cfi, PriceIncrements};
//!
//! let increments = PriceIncrements::value_of("0.01 1; 0.05")?;
//! assert_eq!(increments.round_price(1.234)?, 1.25);
//! let underlyings = AdditionalUnderlyings::value_of("SE 50; FIS 53")?;
//! assert_eq!(underlyings.get_spc("SE")?, 50.0);
//! assert!(Cfi::value_of("OCASPS")?.is_option());
//! # Ok(())
//! # }
//! ```

mod additional_underlyings;
mod cfi;
mod price_increments;

pub use additional_underlyings::AdditionalUnderlyings;
pub use cfi::{Cfi, CfiAttribute, CfiValue};
pub use price_increments::{PriceIncrements, RoundingMode};

use crate::error::Result;
use crate::isolate::{Thread, call};

/// Calls an SDK function that writes a value to its out-pointer and returns the value.
fn out<T: Default>(f: impl FnOnce(Thread, *mut T) -> i32) -> Result<T> {
    let mut value = T::default();
    call(|t| f(t, &mut value))?;
    Ok(value)
}
