//! Live instrument profiles (`com.dxfeed.ipf.live`): a collector of profiles and a connection that fills it from a
//! file or a service and keeps it up to date.

mod collector;
mod connection;

pub use collector::InstrumentProfileCollector;
pub(crate) use collector::collect;
pub use connection::{ConnectionState, InstrumentProfileConnection};
