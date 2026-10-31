//! The sources of indexed events (`IndexedEventSource`, `OrderSource`).

use std::fmt::{self, Debug, Display, Formatter};
use std::str::FromStr;

use super::flags::{SPECIAL_SOURCES, source_name};
use crate::error::{Error, Result};

/// The source of indexed events, such as the exchange or the feed of orders (`IndexedEventSource`, `OrderSource`).
///
/// A source is its id; the name follows from it as in Java's `OrderSource`: [`DEFAULT`](Self::DEFAULT) (id 0), the
/// special sources with ids 1 to 9, such as [`COMPOSITE_BID`](Self::COMPOSITE_BID), and the sources whose one to four
/// ASCII letters and digits are packed big-endian into the id, such as [`NTV`](Self::NTV). The events of all types
/// but the orders have the [`DEFAULT`](Self::DEFAULT) source.
///
/// ```
/// use dxfeed_graal::event::OrderSource;
///
/// assert_eq!(OrderSource::value_of("NTV").unwrap(), OrderSource::NTV);
/// assert_eq!(OrderSource::NTV.to_string(), "NTV");
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct IndexedEventSource {
    id: i32,
}

/// The source of orders: Java's `OrderSource` is the [`IndexedEventSource`] of the order events.
pub type OrderSource = IndexedEventSource;

impl IndexedEventSource {
    /// The default source, of the events of all types but the orders, and of the orders without a source (`DEFAULT`).
    pub const DEFAULT: IndexedEventSource = IndexedEventSource { id: 0 };

    /// The special order source `COMPOSITE_BID` (id 1).
    pub const COMPOSITE_BID: IndexedEventSource = IndexedEventSource { id: 1 };

    /// The special order source `COMPOSITE_ASK` (id 2).
    pub const COMPOSITE_ASK: IndexedEventSource = IndexedEventSource { id: 2 };

    /// The special order source `REGIONAL_BID` (id 3).
    pub const REGIONAL_BID: IndexedEventSource = IndexedEventSource { id: 3 };

    /// The special order source `REGIONAL_ASK` (id 4).
    pub const REGIONAL_ASK: IndexedEventSource = IndexedEventSource { id: 4 };

    /// The special order source `AGGREGATE_BID` (id 5).
    pub const AGGREGATE_BID: IndexedEventSource = IndexedEventSource { id: 5 };

    /// The special order source `AGGREGATE_ASK` (id 6).
    pub const AGGREGATE_ASK: IndexedEventSource = IndexedEventSource { id: 6 };

    /// The special order source `COMPOSITE` (id 7).
    pub const COMPOSITE: IndexedEventSource = IndexedEventSource { id: 7 };

    /// The special order source `REGIONAL` (id 8).
    pub const REGIONAL: IndexedEventSource = IndexedEventSource { id: 8 };

    /// The special order source `AGGREGATE` (id 9).
    pub const AGGREGATE: IndexedEventSource = IndexedEventSource { id: 9 };

    /// The order source `NTV`.
    pub const NTV: IndexedEventSource = IndexedEventSource::packed(b"NTV");

    /// The order source `ntv`.
    #[allow(non_upper_case_globals)]
    pub const ntv: IndexedEventSource = IndexedEventSource::packed(b"ntv");

    /// The order source `NFX`.
    pub const NFX: IndexedEventSource = IndexedEventSource::packed(b"NFX");

    /// The order source `ESPD`.
    pub const ESPD: IndexedEventSource = IndexedEventSource::packed(b"ESPD");

    /// The order source `XNFI`.
    pub const XNFI: IndexedEventSource = IndexedEventSource::packed(b"XNFI");

    /// The order source `ICE`.
    pub const ICE: IndexedEventSource = IndexedEventSource::packed(b"ICE");

    /// The order source `ISE`.
    pub const ISE: IndexedEventSource = IndexedEventSource::packed(b"ISE");

    /// The order source `DEA`.
    pub const DEA: IndexedEventSource = IndexedEventSource::packed(b"DEA");

    /// The order source `DEX`.
    pub const DEX: IndexedEventSource = IndexedEventSource::packed(b"DEX");

    /// The order source `dex`.
    #[allow(non_upper_case_globals)]
    pub const dex: IndexedEventSource = IndexedEventSource::packed(b"dex");

    /// The order source `BYX`.
    pub const BYX: IndexedEventSource = IndexedEventSource::packed(b"BYX");

    /// The order source `BZX`.
    pub const BZX: IndexedEventSource = IndexedEventSource::packed(b"BZX");

    /// The order source `bzx`.
    #[allow(non_upper_case_globals)]
    pub const bzx: IndexedEventSource = IndexedEventSource::packed(b"bzx");

    /// The order source `BATE`.
    pub const BATE: IndexedEventSource = IndexedEventSource::packed(b"BATE");

    /// The order source `CHIX`.
    pub const CHIX: IndexedEventSource = IndexedEventSource::packed(b"CHIX");

    /// The order source `CEUX`.
    pub const CEUX: IndexedEventSource = IndexedEventSource::packed(b"CEUX");

    /// The order source `BXTR`.
    pub const BXTR: IndexedEventSource = IndexedEventSource::packed(b"BXTR");

    /// The order source `IST`.
    pub const IST: IndexedEventSource = IndexedEventSource::packed(b"IST");

    /// The order source `BI20`.
    pub const BI20: IndexedEventSource = IndexedEventSource::packed(b"BI20");

    /// The order source `ABE`.
    pub const ABE: IndexedEventSource = IndexedEventSource::packed(b"ABE");

    /// The order source `FAIR`.
    pub const FAIR: IndexedEventSource = IndexedEventSource::packed(b"FAIR");

    /// The order source `GLBX`.
    pub const GLBX: IndexedEventSource = IndexedEventSource::packed(b"GLBX");

    /// The order source `glbx`.
    #[allow(non_upper_case_globals)]
    pub const glbx: IndexedEventSource = IndexedEventSource::packed(b"glbx");

    /// The order source `ERIS`.
    pub const ERIS: IndexedEventSource = IndexedEventSource::packed(b"ERIS");

    /// The order source `XEUR`.
    pub const XEUR: IndexedEventSource = IndexedEventSource::packed(b"XEUR");

    /// The order source `xeur`.
    #[allow(non_upper_case_globals)]
    pub const xeur: IndexedEventSource = IndexedEventSource::packed(b"xeur");

    /// The order source `CFE`.
    pub const CFE: IndexedEventSource = IndexedEventSource::packed(b"CFE");

    /// The order source `C2OX`.
    pub const C2OX: IndexedEventSource = IndexedEventSource::packed(b"C2OX");

    /// The order source `SMFE`.
    pub const SMFE: IndexedEventSource = IndexedEventSource::packed(b"SMFE");

    /// The order source `smfe`.
    #[allow(non_upper_case_globals)]
    pub const smfe: IndexedEventSource = IndexedEventSource::packed(b"smfe");

    /// The order source `iex`.
    #[allow(non_upper_case_globals)]
    pub const iex: IndexedEventSource = IndexedEventSource::packed(b"iex");

    /// The order source `MEMX`.
    pub const MEMX: IndexedEventSource = IndexedEventSource::packed(b"MEMX");

    /// The order source `memx`.
    #[allow(non_upper_case_globals)]
    pub const memx: IndexedEventSource = IndexedEventSource::packed(b"memx");

    /// The order source `OCEA`.
    pub const OCEA: IndexedEventSource = IndexedEventSource::packed(b"OCEA");

    /// The order source `ocea`.
    #[allow(non_upper_case_globals)]
    pub const ocea: IndexedEventSource = IndexedEventSource::packed(b"ocea");

    /// The order source `pink`.
    #[allow(non_upper_case_globals)]
    pub const pink: IndexedEventSource = IndexedEventSource::packed(b"pink");

    /// The order source `ARCA`.
    pub const ARCA: IndexedEventSource = IndexedEventSource::packed(b"ARCA");

    /// The order source `arca`.
    #[allow(non_upper_case_globals)]
    pub const arca: IndexedEventSource = IndexedEventSource::packed(b"arca");

    /// The order source `CEDX`.
    pub const CEDX: IndexedEventSource = IndexedEventSource::packed(b"CEDX");

    /// The order source `cedx`.
    #[allow(non_upper_case_globals)]
    pub const cedx: IndexedEventSource = IndexedEventSource::packed(b"cedx");

    /// The order source `IGC`.
    pub const IGC: IndexedEventSource = IndexedEventSource::packed(b"IGC");

    /// The order source `igc`.
    #[allow(non_upper_case_globals)]
    pub const igc: IndexedEventSource = IndexedEventSource::packed(b"igc");

    /// The order source `EDX`.
    pub const EDX: IndexedEventSource = IndexedEventSource::packed(b"EDX");

    /// The order source `edx`.
    #[allow(non_upper_case_globals)]
    pub const edx: IndexedEventSource = IndexedEventSource::packed(b"edx");

    /// The order source `NUAM`.
    pub const NUAM: IndexedEventSource = IndexedEventSource::packed(b"NUAM");

    /// The order source `nuam`.
    #[allow(non_upper_case_globals)]
    pub const nuam: IndexedEventSource = IndexedEventSource::packed(b"nuam");

    /// The order source `MOON`.
    pub const MOON: IndexedEventSource = IndexedEventSource::packed(b"MOON");

    /// The order source `moon`.
    #[allow(non_upper_case_globals)]
    pub const moon: IndexedEventSource = IndexedEventSource::packed(b"moon");

    /// The order source `OTCM`.
    pub const OTCM: IndexedEventSource = IndexedEventSource::packed(b"OTCM");

    /// The order source `otcm`.
    #[allow(non_upper_case_globals)]
    pub const otcm: IndexedEventSource = IndexedEventSource::packed(b"otcm");

    /// The order source `BOSS`.
    pub const BOSS: IndexedEventSource = IndexedEventSource::packed(b"BOSS");

    /// The order source `boss`.
    #[allow(non_upper_case_globals)]
    pub const boss: IndexedEventSource = IndexedEventSource::packed(b"boss");

    /// The order source `NEO`.
    pub const NEO: IndexedEventSource = IndexedEventSource::packed(b"NEO");

    /// The order source `neo`.
    #[allow(non_upper_case_globals)]
    pub const neo: IndexedEventSource = IndexedEventSource::packed(b"neo");

    /// The source of `name`, as Java's `OrderSource.valueOf(String)` finds or creates it: `DEFAULT`, a special source
    /// or one to four ASCII letters and digits.
    ///
    /// Returns [`Error::InvalidArgument`] for any other name.
    #[doc(alias = "valueOf")]
    pub fn value_of(name: &str) -> Result<IndexedEventSource> {
        if name == "DEFAULT" {
            return Ok(IndexedEventSource::DEFAULT);
        }
        if let Some(index) = SPECIAL_SOURCES.iter().position(|special| *special == name) {
            return Ok(IndexedEventSource { id: index as i32 + 1 });
        }
        if !(1..=4).contains(&name.len()) || !name.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err(Error::InvalidArgument(format!(
                "the source name {name:?} must have one to four ASCII letters and digits"
            )));
        }
        Ok(IndexedEventSource::packed(name.as_bytes()))
    }

    /// The source with the id `id`, as Java's `OrderSource.valueOf(int)` finds or creates it.
    ///
    /// Returns [`Error::InvalidArgument`] for an id that is neither `0`, a special source nor packed letters and
    /// digits.
    #[doc(alias = "valueOf")]
    pub fn from_id(id: i32) -> Result<IndexedEventSource> {
        match source_name(id) {
            Some(_) => Ok(IndexedEventSource { id }),
            None => Err(Error::InvalidArgument(format!("{id} is not the id of an order source"))),
        }
    }

    /// The id.
    pub fn get_id(self) -> i32 {
        self.id
    }

    /// The name, such as `DEFAULT`, `COMPOSITE_BID` or `NTV`.
    pub fn get_name(self) -> String {
        // Every constructor checks that the id has a name.
        source_name(self.id).unwrap_or_else(|| self.id.to_string())
    }

    /// The source named by `name`, packed big-endian.
    const fn packed(name: &[u8]) -> IndexedEventSource {
        let mut id = 0;
        let mut i = 0;
        while i < name.len() {
            id = id << 8 | name[i] as i32;
            i += 1;
        }
        IndexedEventSource { id }
    }
}

impl Default for IndexedEventSource {
    fn default() -> Self {
        IndexedEventSource::DEFAULT
    }
}

impl Display for IndexedEventSource {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.pad(&self.get_name())
    }
}

impl Debug for IndexedEventSource {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "IndexedEventSource({})", self.get_name())
    }
}

impl FromStr for IndexedEventSource {
    type Err = Error;

    /// Parses a name as [`IndexedEventSource::value_of`] does.
    fn from_str(name: &str) -> Result<Self> {
        IndexedEventSource::value_of(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_ids() {
        assert_eq!(IndexedEventSource::NTV.get_id(), 0x4e5456);
        assert_eq!(IndexedEventSource::value_of("ntv").unwrap(), IndexedEventSource::ntv);
        assert_eq!(IndexedEventSource::value_of("AGGREGATE").unwrap().get_id(), 9);
        assert_eq!(
            IndexedEventSource::value_of("DEFAULT").unwrap(),
            IndexedEventSource::DEFAULT
        );
        assert_eq!(IndexedEventSource::from_id(0x4e5456).unwrap().get_name(), "NTV");
        assert_eq!(IndexedEventSource::BI20.get_name(), "BI20");
        assert_eq!(
            format!("{:?}", IndexedEventSource::COMPOSITE_BID),
            "IndexedEventSource(COMPOSITE_BID)"
        );
        for bad in ["", "TOOLONG", "N-V", "NT\u{e9}"] {
            assert!(IndexedEventSource::value_of(bad).is_err(), "{bad:?}");
        }
        assert!(IndexedEventSource::from_id(-1).is_err());
    }
}
