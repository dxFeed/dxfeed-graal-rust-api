//! Instrument profiles (`com.dxfeed.ipf`): the reference data of instruments in the Instrument Profile Format, read
//! from files and URLs, written to files and memory, followed live with [`live`] and arranged into option chains with
//! [`option`].

pub mod live;
pub mod option;
pub(crate) mod profile;
mod reader;
mod writer;

pub use profile::{InstrumentProfile, InstrumentProfileField, InstrumentProfileType};
pub use reader::InstrumentProfileReader;
pub use writer::InstrumentProfileWriter;
