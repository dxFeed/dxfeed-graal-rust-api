//! Instrument profiles (`com.dxfeed.ipf`): the reference data of instruments in the Instrument Profile Format, read
//! from files and URLs, followed live with [`live`] and arranged into option chains with [`option`].

pub mod live;
pub mod option;
mod profile;
mod reader;

pub use profile::InstrumentProfile;
pub use reader::InstrumentProfileReader;
