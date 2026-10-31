//! Option chains built from instrument profiles (`com.dxfeed.ipf.option`).
//!
//! The SDK has no option chains: this is a port of the Java classes, like the one of the C++ API.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::hash::{Hash, Hasher};

use crate::event::{JavaDouble, year_month_day};
use crate::ipf::InstrumentProfile;

/// A strike as a map key, ordered like Java's `Double.compare`.
#[derive(Debug, Clone, Copy)]
struct Strike(f64);

impl PartialEq for Strike {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Strike {}

impl PartialOrd for Strike {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Strike {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.total_cmp(&other.0)
    }
}

/// The options of one series of a chain: the same expiration and other attributes, by strike (`OptionSeries`).
///
/// Series are equal, hashed and ordered by their attributes alone, not by their options, as in Java: by
/// [`get_expiration`](Self::get_expiration), [`get_last_trade`](Self::get_last_trade), [`get_multiplier`](Self::get_multiplier),
/// [`get_spc`](Self::get_spc), [`get_additional_underlyings`](Self::get_additional_underlyings), [`get_mmy`](Self::get_mmy),
/// [`get_option_type`](Self::get_option_type), [`get_expiration_style`](Self::get_expiration_style),
/// [`get_settlement_style`](Self::get_settlement_style) and [`get_cfi`](Self::get_cfi).
#[derive(Debug, Clone)]
pub struct OptionSeries<T> {
    expiration: i32,
    last_trade: i32,
    multiplier: f64,
    spc: f64,
    additional_underlyings: String,
    mmy: String,
    option_type: String,
    expiration_style: String,
    settlement_style: String,
    cfi: String,
    calls: BTreeMap<Strike, T>,
    puts: BTreeMap<Strike, T>,
    strikes: Vec<f64>,
}

impl<T> OptionSeries<T> {
    /// A series without attributes and options.
    fn new() -> Self {
        OptionSeries {
            expiration: 0,
            last_trade: 0,
            multiplier: 0.0,
            spc: 0.0,
            additional_underlyings: String::new(),
            mmy: String::new(),
            option_type: String::new(),
            expiration_style: String::new(),
            settlement_style: String::new(),
            cfi: String::new(),
            calls: BTreeMap::new(),
            puts: BTreeMap::new(),
            strikes: Vec::new(),
        }
    }
}

impl<T> OptionSeries<T> {
    /// The day id of the expiration, the days since 1970-01-01 (`getExpiration`).
    #[doc(alias = "getExpiration")]
    pub fn get_expiration(&self) -> i32 {
        self.expiration
    }

    /// The day id of the last trading day, the days since 1970-01-01 (`getLastTrade`).
    #[doc(alias = "getLastTrade")]
    pub fn get_last_trade(&self) -> i32 {
        self.last_trade
    }

    /// The market value multiplier (`getMultiplier`).
    #[doc(alias = "getMultiplier")]
    pub fn get_multiplier(&self) -> f64 {
        self.multiplier
    }

    /// The shares per contract of each option (`getSPC`).
    #[doc(alias = "getSPC")]
    pub fn get_spc(&self) -> f64 {
        self.spc
    }

    /// The additional underlyings of the options, such as `SE=50; FIS=53` (`getAdditionalUnderlyings`).
    #[doc(alias = "getAdditionalUnderlyings")]
    pub fn get_additional_underlyings(&self) -> &str {
        &self.additional_underlyings
    }

    /// The maturity month-year of the options as in FIX, such as `201202` or `20110817` (`getMMY`).
    #[doc(alias = "getMMY")]
    pub fn get_mmy(&self) -> &str {
        &self.mmy
    }

    /// The type of the options, such as `STAN` or `LEAP` (`getOptionType`).
    #[doc(alias = "getOptionType")]
    pub fn get_option_type(&self) -> &str {
        &self.option_type
    }

    /// The expiration style of the options, such as `Weeklys` or `Quarterlys` (`getExpirationStyle`).
    #[doc(alias = "getExpirationStyle")]
    pub fn get_expiration_style(&self) -> &str {
        &self.expiration_style
    }

    /// The settlement price determination style, such as `Open` or `Close` (`getSettlementStyle`).
    #[doc(alias = "getSettlementStyle")]
    pub fn get_settlement_style(&self) -> &str {
        &self.settlement_style
    }

    /// The CFI code of the options with `X` for the call or put attribute, such as `OXXXXX` (`getCFI`).
    #[doc(alias = "getCFI")]
    pub fn get_cfi(&self) -> &str {
        &self.cfi
    }

    /// The calls by strike, in ascending order of strikes (`getCalls`).
    #[doc(alias = "getCalls")]
    pub fn get_calls(&self) -> impl ExactSizeIterator<Item = (f64, &T)> + DoubleEndedIterator {
        self.calls.iter().map(|(strike, option)| (strike.0, option))
    }

    /// The puts by strike, in ascending order of strikes (`getPuts`).
    #[doc(alias = "getPuts")]
    pub fn get_puts(&self) -> impl ExactSizeIterator<Item = (f64, &T)> + DoubleEndedIterator {
        self.puts.iter().map(|(strike, option)| (strike.0, option))
    }

    /// The call of a strike (`getCalls().get(strike)`).
    pub fn call(&self, strike: f64) -> Option<&T> {
        self.calls.get(&Strike(strike))
    }

    /// The put of a strike (`getPuts().get(strike)`).
    pub fn put(&self, strike: f64) -> Option<&T> {
        self.puts.get(&Strike(strike))
    }

    /// The strikes of the calls and the puts, in ascending order (`getStrikes`).
    #[doc(alias = "getStrikes")]
    pub fn get_strikes(&self) -> &[f64] {
        &self.strikes
    }

    /// At most `n` strikes around `strike`, with the nearest strike at or above it in the middle (`getNStrikesAround`).
    ///
    /// Fewer strikes are returned near the ends: when all strikes are below `strike`, only the last `n / 2` of them.
    #[doc(alias = "getNStrikesAround")]
    pub fn get_n_strikes_around(&self, n: usize, strike: f64) -> &[f64] {
        let index = self.strikes.partition_point(|s| s.total_cmp(&strike).is_lt());
        let from = index.saturating_sub(n / 2);
        let to = self.strikes.len().min(from + n);
        &self.strikes[from..to]
    }

    /// A series with the same attributes and no options.
    fn attributes(&self) -> Self {
        OptionSeries {
            expiration: self.expiration,
            last_trade: self.last_trade,
            multiplier: self.multiplier,
            spc: self.spc,
            additional_underlyings: self.additional_underlyings.clone(),
            mmy: self.mmy.clone(),
            option_type: self.option_type.clone(),
            expiration_style: self.expiration_style.clone(),
            settlement_style: self.settlement_style.clone(),
            cfi: self.cfi.clone(),
            ..OptionSeries::new()
        }
    }

    /// Adds an option, replacing the one of the same kind and strike, as Java does.
    fn add_option(&mut self, is_call: bool, strike: f64, option: T) {
        let map = if is_call { &mut self.calls } else { &mut self.puts };
        if map.insert(Strike(strike), option).is_none()
            && let Err(index) = self.strikes.binary_search_by(|s| s.total_cmp(&strike))
        {
            self.strikes.insert(index, strike);
        }
    }

    fn compare(&self, other: &Self) -> Ordering {
        self.expiration
            .cmp(&other.expiration)
            .then(self.last_trade.cmp(&other.last_trade))
            .then(self.multiplier.total_cmp(&other.multiplier))
            .then(self.spc.total_cmp(&other.spc))
            .then_with(|| self.additional_underlyings.cmp(&other.additional_underlyings))
            .then_with(|| self.mmy.cmp(&other.mmy))
            .then_with(|| self.option_type.cmp(&other.option_type))
            .then_with(|| self.expiration_style.cmp(&other.expiration_style))
            .then_with(|| self.settlement_style.cmp(&other.settlement_style))
            .then_with(|| self.cfi.cmp(&other.cfi))
    }
}

impl<T> PartialEq for OptionSeries<T> {
    fn eq(&self, other: &Self) -> bool {
        self.compare(other) == Ordering::Equal
    }
}

impl<T> Eq for OptionSeries<T> {}

impl<T> PartialOrd for OptionSeries<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T> Ord for OptionSeries<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.compare(other)
    }
}

impl<T> Hash for OptionSeries<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.expiration.hash(state);
        self.last_trade.hash(state);
        self.multiplier.to_bits().hash(state);
        self.spc.to_bits().hash(state);
        self.additional_underlyings.hash(state);
        self.mmy.hash(state);
        self.option_type.hash(state);
        self.expiration_style.hash(state);
        self.settlement_style.hash(state);
        self.cfi.hash(state);
    }
}

impl<T> fmt::Display for OptionSeries<T> {
    /// The attributes as Java's `toString` prints them, such as `expiration=20261218, spc=100.0, cfi=OXXXXX`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "expiration={}", year_month_day(self.expiration))?;
        if self.last_trade != 0 {
            write!(f, ", lastTrade={}", year_month_day(self.last_trade))?;
        }
        if self.multiplier != 0.0 {
            write!(f, ", multiplier={}", JavaDouble(self.multiplier))?;
        }
        if self.spc != 0.0 {
            write!(f, ", spc={}", JavaDouble(self.spc))?;
        }
        for (name, value) in [
            ("additionalUnderlyings", &self.additional_underlyings),
            ("mmy", &self.mmy),
            ("optionType", &self.option_type),
            ("expirationStyle", &self.expiration_style),
            ("settlementStyle", &self.settlement_style),
        ] {
            if !value.is_empty() {
                write!(f, ", {name}={value}")?;
            }
        }
        write!(f, ", cfi={}", self.cfi)
    }
}

/// The option series of one product or underlying symbol (`OptionChain`).
#[derive(Debug, Clone)]
pub struct OptionChain<T> {
    symbol: String,
    series: Vec<OptionSeries<T>>,
}

impl<T> OptionChain<T> {
    fn new(symbol: &str) -> Self {
        OptionChain {
            symbol: symbol.to_owned(),
            series: Vec::new(),
        }
    }

    /// The product or underlying symbol of the chain (`getSymbol`).
    #[doc(alias = "getSymbol")]
    pub fn get_symbol(&self) -> &str {
        &self.symbol
    }

    /// The series of the chain, in ascending order of their attributes, the nearest expiration first (`getSeries`).
    #[doc(alias = "getSeries")]
    pub fn get_series(&self) -> &[OptionSeries<T>] {
        &self.series
    }

    fn add_option(&mut self, series: &OptionSeries<T>, is_call: bool, strike: f64, option: T) {
        let index = match self.series.binary_search(series) {
            Ok(index) => index,
            Err(index) => {
                self.series.insert(index, series.attributes());
                index
            }
        };
        self.series[index].add_option(is_call, strike, option);
    }
}

/// Builds option chains: the attributes of an option are set one by one, then [`add_option`](Self::add_option) adds
/// it to the chains of its product and of its underlying (`OptionChainsBuilder`).
///
/// [`build`](OptionChainsBuilder::build) does all of it for instrument profiles. An option is skipped when its CFI
/// code does not start with `OC` (a call) or `OP` (a put), its expiration is not set or its strike is not finite.
///
/// ```no_run
/// use dxfeed_graal::ipf::InstrumentProfileReader;
/// use dxfeed_graal::ipf::option::OptionChainsBuilder;
///
/// let profiles = InstrumentProfileReader::new()?.read_from_file("profiles.ipf.zip")?;
/// let chains = OptionChainsBuilder::build(&profiles).into_chains();
/// for series in chains["AAPL"].get_series() {
///     println!("{series}: {} strikes", series.get_strikes().len());
/// }
/// # Ok::<(), dxfeed_graal::Error>(())
/// ```
#[derive(Debug, Clone)]
pub struct OptionChainsBuilder<T> {
    product: String,
    underlying: String,
    series: OptionSeries<T>,
    cfi: String,
    strike: f64,
    chains: HashMap<String, OptionChain<T>>,
}

impl<T> Default for OptionChainsBuilder<T> {
    fn default() -> Self {
        OptionChainsBuilder {
            product: String::new(),
            underlying: String::new(),
            series: OptionSeries::new(),
            cfi: String::new(),
            strike: 0.0,
            chains: HashMap::new(),
        }
    }
}

impl<'a> OptionChainsBuilder<&'a InstrumentProfile> {
    /// Builds the chains of the options among `instruments`, the profiles of the type `OPTION` (`build`).
    ///
    /// Like in Java, it returns a builder that holds the chains: [`get_chains`](Self::get_chains) or
    /// [`into_chains`](Self::into_chains) give them. The chains refer to the profiles, which are not copied.
    pub fn build(instruments: impl IntoIterator<Item = &'a InstrumentProfile>) -> Self {
        let mut builder = OptionChainsBuilder::new();
        for profile in instruments {
            if profile.instrument_type != "OPTION" {
                continue;
            }
            builder.set_product(&profile.product);
            builder.set_underlying(&profile.underlying);
            builder.set_expiration(profile.expiration);
            builder.set_last_trade(profile.last_trade);
            builder.set_multiplier(profile.multiplier);
            builder.set_spc(profile.spc);
            builder.set_additional_underlyings(&profile.additional_underlyings);
            builder.set_mmy(&profile.mmy);
            builder.set_option_type(&profile.option_type);
            builder.set_expiration_style(&profile.expiration_style);
            builder.set_settlement_style(&profile.settlement_style);
            builder.set_cfi(&profile.cfi);
            builder.set_strike(profile.strike);
            builder.add_option(profile);
        }
        builder
    }
}

impl<T> OptionChainsBuilder<T> {
    /// Creates a builder without chains.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the product of the next option, such as `/YG`; empty for none (`setProduct`).
    #[doc(alias = "setProduct")]
    pub fn set_product(&mut self, product: &str) {
        product.clone_into(&mut self.product);
    }

    /// Sets the underlying of the next option, such as `AAPL`; empty for none (`setUnderlying`).
    #[doc(alias = "setUnderlying")]
    pub fn set_underlying(&mut self, underlying: &str) {
        underlying.clone_into(&mut self.underlying);
    }

    /// Sets the day id of the expiration of the next option (`setExpiration`).
    #[doc(alias = "setExpiration")]
    pub fn set_expiration(&mut self, expiration: i32) {
        self.series.expiration = expiration;
    }

    /// Sets the day id of the last trading day of the next option (`setLastTrade`).
    #[doc(alias = "setLastTrade")]
    pub fn set_last_trade(&mut self, last_trade: i32) {
        self.series.last_trade = last_trade;
    }

    /// Sets the market value multiplier of the next option (`setMultiplier`).
    #[doc(alias = "setMultiplier")]
    pub fn set_multiplier(&mut self, multiplier: f64) {
        self.series.multiplier = multiplier;
    }

    /// Sets the shares per contract of the next option (`setSPC`).
    #[doc(alias = "setSPC")]
    pub fn set_spc(&mut self, spc: f64) {
        self.series.spc = spc;
    }

    /// Sets the additional underlyings of the next option (`setAdditionalUnderlyings`).
    #[doc(alias = "setAdditionalUnderlyings")]
    pub fn set_additional_underlyings(&mut self, additional_underlyings: &str) {
        additional_underlyings.clone_into(&mut self.series.additional_underlyings);
    }

    /// Sets the maturity month-year of the next option (`setMMY`).
    #[doc(alias = "setMMY")]
    pub fn set_mmy(&mut self, mmy: &str) {
        mmy.clone_into(&mut self.series.mmy);
    }

    /// Sets the type of the next option, such as `STAN` (`setOptionType`).
    #[doc(alias = "setOptionType")]
    pub fn set_option_type(&mut self, option_type: &str) {
        option_type.clone_into(&mut self.series.option_type);
    }

    /// Sets the expiration style of the next option (`setExpirationStyle`).
    #[doc(alias = "setExpirationStyle")]
    pub fn set_expiration_style(&mut self, expiration_style: &str) {
        expiration_style.clone_into(&mut self.series.expiration_style);
    }

    /// Sets the settlement price determination style of the next option (`setSettlementStyle`).
    #[doc(alias = "setSettlementStyle")]
    pub fn set_settlement_style(&mut self, settlement_style: &str) {
        settlement_style.clone_into(&mut self.series.settlement_style);
    }

    /// Sets the CFI code of the next option, such as `OCASPS`: `OC` marks a call and `OP` a put. Its series gets
    /// the code with `X` in the place of the second letter (`setCFI`).
    #[doc(alias = "setCFI")]
    pub fn set_cfi(&mut self, cfi: &str) {
        cfi.clone_into(&mut self.cfi);
        let mut chars = cfi.chars();
        self.series.cfi = match (chars.next(), chars.next()) {
            (Some(first), Some(_)) => format!("{first}X{}", chars.as_str()),
            _ => cfi.to_owned(),
        };
    }

    /// Sets the strike of the next option (`setStrike`).
    #[doc(alias = "setStrike")]
    pub fn set_strike(&mut self, strike: f64) {
        self.strike = strike;
    }

    /// The chains built so far by product or underlying symbol (`getChains`).
    #[doc(alias = "getChains")]
    pub fn get_chains(&self) -> &HashMap<String, OptionChain<T>> {
        &self.chains
    }

    /// Takes the chains built, by product or underlying symbol (`getChains`).
    pub fn into_chains(self) -> HashMap<String, OptionChain<T>> {
        self.chains
    }
}

impl<T: Clone> OptionChainsBuilder<T> {
    /// Adds an option with the attributes set before to the chains of its product and of its underlying; skips it
    /// when it is neither a call nor a put, has no expiration or no finite strike (`addOption`).
    #[doc(alias = "addOption")]
    pub fn add_option(&mut self, option: T) {
        let is_call = self.cfi.starts_with("OC");
        if !is_call && !self.cfi.starts_with("OP") || self.series.expiration == 0 || !self.strike.is_finite() {
            return;
        }
        if !self.product.is_empty() {
            add_to_chain(
                &mut self.chains,
                &self.product,
                &self.series,
                is_call,
                self.strike,
                option.clone(),
            );
        }
        if !self.underlying.is_empty() {
            add_to_chain(
                &mut self.chains,
                &self.underlying,
                &self.series,
                is_call,
                self.strike,
                option,
            );
        }
    }
}

fn add_to_chain<T>(
    chains: &mut HashMap<String, OptionChain<T>>,
    symbol: &str,
    series: &OptionSeries<T>,
    is_call: bool,
    strike: f64,
    option: T,
) {
    let chain = chains
        .entry(symbol.to_owned())
        .or_insert_with(|| OptionChain::new(symbol));
    chain.add_option(series, is_call, strike, option);
}
