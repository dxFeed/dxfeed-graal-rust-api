//! The sources of indexed events (`IndexedEventSource`, `OrderSource`).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Debug, Display, Formatter};
use std::hash::{Hash, Hasher};
use std::str::FromStr;
use std::sync::{Mutex, PoisonError};

use dxfeed_graal_sys as sys;

use super::EventType;
use super::flags::{SPECIAL_SOURCES, source_name};
use crate::error::{Error, Result};

/// The source of indexed events, such as the exchange or the feed of orders (`IndexedEventSource`, `OrderSource`).
///
/// A source is its id; the name follows from it as in Java's `OrderSource`: [`DEFAULT`](Self::DEFAULT) (id 0), the
/// special sources with ids 1 to 9, such as [`COMPOSITE_BID`](Self::COMPOSITE_BID), and the sources whose one to four
/// ASCII letters and digits are packed big-endian into the id, such as [`NTV`](Self::NTV). Any other id gets its name
/// from [`new`](Self::new). The events of all types but the orders and [`OrderImbalance`](super::OrderImbalance) have
/// the [`DEFAULT`](Self::DEFAULT) source.
///
/// Two sources are equal when their ids are, whatever their names, as in Java.
///
/// ```
/// use dxfeed_graal::event::{EventType, OrderSource};
///
/// assert_eq!(OrderSource::value_of("NTV").unwrap(), OrderSource::NTV);
/// assert_eq!(OrderSource::NTV.to_string(), "NTV");
/// assert!(OrderSource::NTV.is_full_order_book());
/// assert!(OrderSource::ISE.is_publishable(EventType::SpreadOrder).unwrap());
/// ```
#[derive(Clone, Copy)]
pub struct IndexedEventSource {
    id: i32,
    /// The name given to [`new`](Self::new); `None` for a name that follows from the id.
    name: Option<&'static str>,
}

/// The names given to [`IndexedEventSource::new`]: each distinct name is leaked once, and the first name of an id is
/// the name [`IndexedEventSource::from_id`] gives it.
struct Registry {
    names: BTreeSet<&'static str>,
    by_id: BTreeMap<i32, &'static str>,
}

static REGISTRY: Mutex<Registry> = Mutex::new(Registry {
    names: BTreeSet::new(),
    by_id: BTreeMap::new(),
});

/// The publication flags of Java's `OrderSource`: the event types published on a source and whether it is a full
/// order book.
const PUB_ORDER: u8 = 0x01;
const PUB_ANALYTIC_ORDER: u8 = 0x02;
const PUB_OTC_MARKETS_ORDER: u8 = 0x04;
const PUB_SPREAD_ORDER: u8 = 0x08;
const FULL_ORDER_BOOK: u8 = 0x10;
const PUB_NUAM_ORDER: u8 = 0x20;
const PUB_ORDER_IMBALANCE: u8 = 0x40;

/// The source of orders: Java's `OrderSource` is the [`IndexedEventSource`] of the order events.
pub type OrderSource = IndexedEventSource;

impl IndexedEventSource {
    /// The default source, of the events of all types but the orders, and of the orders without a source (`DEFAULT`).
    pub const DEFAULT: IndexedEventSource = IndexedEventSource::of(0);

    /// The special order source `COMPOSITE_BID` (id 1).
    pub const COMPOSITE_BID: IndexedEventSource = IndexedEventSource::of(1);

    /// The special order source `COMPOSITE_ASK` (id 2).
    pub const COMPOSITE_ASK: IndexedEventSource = IndexedEventSource::of(2);

    /// The special order source `REGIONAL_BID` (id 3).
    pub const REGIONAL_BID: IndexedEventSource = IndexedEventSource::of(3);

    /// The special order source `REGIONAL_ASK` (id 4).
    pub const REGIONAL_ASK: IndexedEventSource = IndexedEventSource::of(4);

    /// The special order source `AGGREGATE_BID` (id 5).
    pub const AGGREGATE_BID: IndexedEventSource = IndexedEventSource::of(5);

    /// The special order source `AGGREGATE_ASK` (id 6).
    pub const AGGREGATE_ASK: IndexedEventSource = IndexedEventSource::of(6);

    /// The special order source `COMPOSITE` (id 7).
    pub const COMPOSITE: IndexedEventSource = IndexedEventSource::of(7);

    /// The special order source `REGIONAL` (id 8).
    pub const REGIONAL: IndexedEventSource = IndexedEventSource::of(8);

    /// The special order source `AGGREGATE` (id 9).
    pub const AGGREGATE: IndexedEventSource = IndexedEventSource::of(9);

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
            return Ok(IndexedEventSource::of(index as i32 + 1));
        }
        if !(1..=4).contains(&name.len()) || !name.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err(Error::InvalidArgument(format!(
                "the source name {name:?} must have one to four ASCII letters and digits"
            )));
        }
        Ok(IndexedEventSource::packed(name.as_bytes()))
    }

    /// The source with the id `id`, as Java's `OrderSource.valueOf(int)` finds or creates it, or the source that
    /// [`new`](Self::new) created first with this id.
    ///
    /// Returns [`Error::InvalidArgument`] for an id that is neither `0`, a special source, packed letters and digits
    /// nor given to [`new`](Self::new).
    #[doc(alias = "valueOf")]
    pub fn from_id(id: i32) -> Result<IndexedEventSource> {
        if source_name(id).is_some() {
            return Ok(IndexedEventSource::of(id));
        }
        match registry().by_id.get(&id) {
            Some(&name) => Ok(IndexedEventSource { id, name: Some(name) }),
            None => Err(Error::InvalidArgument(format!("{id} is not the id of an order source"))),
        }
    }

    /// A source with the id `id` and the name `name`, as Java's `new IndexedEventSource(id, name)` creates it.
    ///
    /// The name is kept for the whole process. For an id that Java does not name, [`from_id`](Self::from_id) then
    /// accepts the id and gives the first name given to it, which an order with the id also prints. A source equals any
    /// other with the same id, such as [`AGGREGATE_BID`](Self::AGGREGATE_BID) for the id `5`, whatever the names; the
    /// SDK gets such a source by its id, as Java's `OrderSource`.
    pub fn new(id: i32, name: &str) -> IndexedEventSource {
        let mut registry = registry();
        let name = match registry.names.get(name) {
            Some(&name) => name,
            None => {
                let name: &'static str = Box::leak(Box::<str>::from(name));
                registry.names.insert(name);
                name
            }
        };
        registry.by_id.entry(id).or_insert(name);
        IndexedEventSource { id, name: Some(name) }
    }

    /// The id.
    #[doc(alias = "id")]
    pub fn get_id(self) -> i32 {
        self.id
    }

    /// The name, such as `DEFAULT`, `COMPOSITE_BID` or `NTV`, the name given to [`new`](Self::new), or the decimal id
    /// of an unknown source.
    #[doc(alias = "name")]
    pub fn get_name(self) -> String {
        if let Some(name) = self.name {
            return name.to_owned();
        }
        source_name(self.id)
            .or_else(|| registry().by_id.get(&self.id).map(|&name| name.to_owned()))
            .unwrap_or_else(|| self.id.to_string())
    }

    /// Tells whether `id` is the id of a special source, [`COMPOSITE_BID`](Self::COMPOSITE_BID) to
    /// [`AGGREGATE`](Self::AGGREGATE): `1` to `9`. Orders keep these ids in bits 48 to 63 of the index, the others in
    /// bits 32 to 63.
    #[doc(alias = "isSpecialSourceId")]
    pub fn is_special_source_id(id: i32) -> bool {
        (1..=9).contains(&id)
    }

    /// The sources on which events of `event_type` can be published, in the order of Java's
    /// `OrderSource.publishable`: [`DEFAULT`](Self::DEFAULT) first, then the named sources of Java.
    ///
    /// Returns [`Error::InvalidArgument`] for a type other than [`Order`](EventType::Order),
    /// [`AnalyticOrder`](EventType::AnalyticOrder), [`OtcMarketsOrder`](EventType::OtcMarketsOrder),
    /// [`SpreadOrder`](EventType::SpreadOrder), [`NuamOrder`](EventType::NuamOrder) and
    /// [`OrderImbalance`](EventType::OrderImbalance).
    pub fn publishable(event_type: EventType) -> Result<Vec<IndexedEventSource>> {
        Ok(with_flags(publication_flag(event_type)?))
    }

    /// The sources that give the full order book, in the order of Java's `OrderSource.fullOrderBook`, such as
    /// [`NTV`](Self::NTV).
    #[doc(alias = "fullOrderBook")]
    pub fn full_order_book() -> Vec<IndexedEventSource> {
        with_flags(FULL_ORDER_BOOK)
    }

    /// Tells whether events of `event_type` can be published on this source, as in Java: only the named sources of
    /// Java and [`DEFAULT`](Self::DEFAULT) are publishable.
    ///
    /// Returns [`Error::InvalidArgument`] for the types that [`publishable`](Self::publishable) refuses.
    #[doc(alias = "isPublishable")]
    pub fn is_publishable(self, event_type: EventType) -> Result<bool> {
        Ok(self.flags() & publication_flag(event_type)? != 0)
    }

    /// Tells whether this source gives the full order book, as [`NTV`](Self::NTV) and [`DEFAULT`](Self::DEFAULT) do.
    #[doc(alias = "isFullOrderBook")]
    pub fn is_full_order_book(self) -> bool {
        self.flags() & FULL_ORDER_BOOK != 0
    }

    /// The source with the id `id` and the name that follows from it, unchecked.
    pub(crate) const fn of(id: i32) -> IndexedEventSource {
        IndexedEventSource { id, name: None }
    }

    /// The name of Java's `OrderSource` with this id, as `OrderSource.valueOf(int)` gives it; `None` for an id that
    /// Java refuses.
    pub(crate) fn order_source_name(self) -> Option<String> {
        source_name(self.id)
    }

    /// Tells whether an order index can keep this source: special sources go to bits 48 to 63, the others to bits
    /// 32 to 63, so bits 16 to 31 of the others must not read as a special source.
    pub(crate) fn fits_order_index(self) -> bool {
        self.id >= 0 && (Self::is_special_source_id(self.id) || !Self::is_special_source_id(self.id >> 16))
    }

    /// The source of an SDK struct: the source of [`from_id`](Self::from_id), or a source with the name of the struct
    /// for an id that is neither named by Java nor given to [`new`](Self::new).
    ///
    /// # Safety
    ///
    /// The name of `source` must be NULL or a live NUL-terminated string.
    pub(crate) unsafe fn from_sys(source: &sys::dxfg_indexed_event_source_t) -> IndexedEventSource {
        if let Ok(known) = IndexedEventSource::from_id(source.id) {
            return known;
        }
        if source.name.is_null() {
            return IndexedEventSource::of(source.id);
        }
        // SAFETY: the caller guarantees that a non-NULL name is live and NUL-terminated.
        IndexedEventSource::new(source.id, &unsafe { crate::isolate::string(source.name) })
    }

    /// The source named by `name`, packed big-endian.
    const fn packed(name: &[u8]) -> IndexedEventSource {
        let mut id = 0;
        let mut i = 0;
        while i < name.len() {
            id = id << 8 | name[i] as i32;
            i += 1;
        }
        IndexedEventSource::of(id)
    }

    /// The publication flags of Java's `OrderSource` with this id; `0` for a source Java does not name.
    fn flags(self) -> u8 {
        BUILTIN
            .iter()
            .find(|(source, _)| source.id == self.id)
            .map_or(0, |&(_, flags)| flags)
    }
}

/// The sources of Java's `OrderSource` with publication flags, in the order of Java's declarations.
const BUILTIN: &[(IndexedEventSource, u8)] = {
    type S = IndexedEventSource;
    const ALL_FLAGS: u8 = PUB_ORDER
        | PUB_ANALYTIC_ORDER
        | PUB_OTC_MARKETS_ORDER
        | PUB_SPREAD_ORDER
        | FULL_ORDER_BOOK
        | PUB_NUAM_ORDER
        | PUB_ORDER_IMBALANCE;
    &[
        (S::DEFAULT, ALL_FLAGS),
        (S::NTV, PUB_ORDER | FULL_ORDER_BOOK | PUB_ORDER_IMBALANCE),
        (S::ntv, PUB_ORDER),
        (S::NFX, PUB_ORDER),
        (S::ESPD, PUB_ORDER),
        (S::XNFI, PUB_ORDER),
        (S::ICE, PUB_ORDER),
        (S::ISE, PUB_ORDER | PUB_SPREAD_ORDER),
        (S::DEA, PUB_ORDER),
        (S::DEX, PUB_ORDER),
        (S::dex, PUB_ORDER),
        (S::BYX, PUB_ORDER),
        (S::BZX, PUB_ORDER),
        (S::bzx, PUB_ORDER),
        (S::BATE, PUB_ORDER),
        (S::CHIX, PUB_ORDER),
        (S::CEUX, PUB_ORDER),
        (S::BXTR, PUB_ORDER),
        (S::IST, PUB_ORDER),
        (S::BI20, PUB_ORDER),
        (S::ABE, PUB_ORDER),
        (S::FAIR, PUB_ORDER),
        (S::GLBX, PUB_ORDER | PUB_ANALYTIC_ORDER),
        (S::glbx, PUB_ORDER),
        (S::ERIS, PUB_ORDER),
        (S::XEUR, PUB_ORDER),
        (S::xeur, PUB_ORDER),
        (S::CFE, PUB_ORDER),
        (S::C2OX, PUB_ORDER),
        (S::SMFE, PUB_ORDER),
        (S::smfe, PUB_ORDER),
        (S::iex, PUB_ORDER),
        (S::MEMX, PUB_ORDER),
        (S::memx, PUB_ORDER),
        (S::OCEA, PUB_ORDER),
        (S::ocea, PUB_ORDER),
        (S::pink, PUB_ORDER | PUB_OTC_MARKETS_ORDER),
        (S::ARCA, PUB_ORDER | PUB_ORDER_IMBALANCE),
        (S::arca, PUB_ORDER),
        (S::CEDX, PUB_ORDER),
        (S::cedx, PUB_ORDER),
        (S::IGC, PUB_ORDER),
        (S::igc, PUB_ORDER),
        (S::EDX, PUB_ORDER),
        (S::edx, PUB_ORDER),
        (
            S::NUAM,
            PUB_ORDER | FULL_ORDER_BOOK | PUB_NUAM_ORDER | PUB_ORDER_IMBALANCE,
        ),
        (S::nuam, PUB_ORDER | PUB_NUAM_ORDER),
        (S::MOON, PUB_ORDER),
        (S::moon, PUB_ORDER),
        (S::OTCM, PUB_ORDER),
        (S::otcm, PUB_ORDER),
        (S::BOSS, PUB_ORDER),
        (S::boss, PUB_ORDER),
        (S::NEO, PUB_ORDER),
        (S::neo, PUB_ORDER),
    ]
};

/// The publication flag of `event_type`, as Java's `OrderSource.getEventTypeMask` gives it.
fn publication_flag(event_type: EventType) -> Result<u8> {
    match event_type {
        EventType::Order => Ok(PUB_ORDER),
        EventType::AnalyticOrder => Ok(PUB_ANALYTIC_ORDER),
        EventType::OtcMarketsOrder => Ok(PUB_OTC_MARKETS_ORDER),
        EventType::SpreadOrder => Ok(PUB_SPREAD_ORDER),
        EventType::NuamOrder => Ok(PUB_NUAM_ORDER),
        EventType::OrderImbalance => Ok(PUB_ORDER_IMBALANCE),
        _ => Err(Error::InvalidArgument(format!("Invalid event type: {event_type}"))),
    }
}

/// The sources of Java with `flag`, in the order of Java's declarations.
fn with_flags(flag: u8) -> Vec<IndexedEventSource> {
    BUILTIN
        .iter()
        .filter(|(_, flags)| flags & flag != 0)
        .map(|&(source, _)| source)
        .collect()
}

/// The names given to [`IndexedEventSource::new`]; a panic elsewhere leaves them valid.
fn registry() -> std::sync::MutexGuard<'static, Registry> {
    REGISTRY.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Default for IndexedEventSource {
    fn default() -> Self {
        IndexedEventSource::DEFAULT
    }
}

impl PartialEq for IndexedEventSource {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for IndexedEventSource {}

impl Hash for IndexedEventSource {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
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

    #[test]
    fn order_indexes_keep_sources_without_special_bits() {
        assert!(IndexedEventSource::NTV.fits_order_index());
        assert!(IndexedEventSource::AGGREGATE.fits_order_index());
        assert!(IndexedEventSource::of(0x0100_0000).fits_order_index());
        assert!(!IndexedEventSource::of(0x1_0000).fits_order_index());
        assert!(!IndexedEventSource::of(0x9_ffff).fits_order_index());
        assert!(IndexedEventSource::of(0xa_0000).fits_order_index());
        assert!(!IndexedEventSource::of(-1).fits_order_index());
    }

    /// The type, the id and the name of the SDK struct of `source`.
    fn sys_of(source: IndexedEventSource) -> (sys::dxfg_indexed_event_source_type_t, i32, String) {
        let mut arena = crate::symbol::SymbolArena::new();
        // SAFETY: the arena keeps the struct and its name alive while it lives.
        let native = unsafe { &*arena.source(source) };
        // SAFETY: the name is a live NUL-terminated string of the arena.
        let name = unsafe { crate::isolate::string(native.name) }.into_owned();
        (native.type_, native.id, name)
    }

    /// The source of an SDK struct with the id `id` and the name `name`.
    fn from_sys(id: i32, name: Option<&str>) -> IndexedEventSource {
        let name = name.map(|name| std::ffi::CString::new(name).unwrap());
        let native = sys::dxfg_indexed_event_source_t {
            type_: sys::INDEXED_EVENT_SOURCE,
            id,
            name: name.as_ref().map_or(std::ptr::null(), |name| name.as_ptr()),
        };
        // SAFETY: the name is NULL or a live NUL-terminated string.
        unsafe { IndexedEventSource::from_sys(&native) }
    }

    #[test]
    fn sdk_structs_carry_the_java_kind_of_source() {
        assert_eq!(
            sys_of(IndexedEventSource::NTV),
            (sys::ORDER_SOURCE, 0x4e5456, "NTV".to_owned())
        );
        assert_eq!(sys_of(IndexedEventSource::DEFAULT).0, sys::INDEXED_EVENT_SOURCE);

        // Java finds an `OrderSource` by its name: a renamed source of Java goes with the name of its id.
        assert_eq!(
            sys_of(IndexedEventSource::new(5, "Renamed")),
            (sys::ORDER_SOURCE, 5, "AGGREGATE_BID".to_owned())
        );
        assert_eq!(
            sys_of(IndexedEventSource::new(0x0200_0000, "My source")),
            (sys::INDEXED_EVENT_SOURCE, 0x0200_0000, "My source".to_owned())
        );
    }

    #[test]
    fn sdk_structs_give_back_their_names() {
        let known = from_sys(0x4e5456, Some("From SDK"));
        assert_eq!((known, known.get_name()), (IndexedEventSource::NTV, "NTV".to_owned()));
        let custom = from_sys(0x0300_0000, Some("From SDK"));
        assert_eq!(
            (custom.get_id(), custom.get_name()),
            (0x0300_0000, "From SDK".to_owned())
        );
        assert_eq!(IndexedEventSource::from_id(0x0300_0000).unwrap().get_name(), "From SDK");
        assert_eq!(from_sys(0x0400_0000, None).get_name(), "67108864");
    }
}
