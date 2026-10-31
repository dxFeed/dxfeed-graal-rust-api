//! Instrument profiles (`com.dxfeed.ipf`): the reference data of instruments in the Instrument Profile Format, read
//! from files and URLs, and followed live with [`live`].

pub mod live;
mod profile;
mod reader;

pub use profile::InstrumentProfile;
pub use reader::InstrumentProfileReader;
