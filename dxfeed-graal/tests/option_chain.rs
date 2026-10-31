//! Option chains built from the profiles of a file and by hand; no network needed.

use std::collections::HashSet;
use std::path::PathBuf;

use dxfeed_graal::ipf::InstrumentProfileReader;
use dxfeed_graal::ipf::option::{OptionChainsBuilder, OptionSeries};

fn fixture() -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/options.ipf")
        .to_string_lossy()
        .into_owned()
}

/// The day ids of 2026-11-20 and 2026-12-18.
const NOVEMBER_20_2026: i32 = 20777;
const DECEMBER_18_2026: i32 = 20805;

fn symbols<'a, T: AsRef<str> + 'a>(options: impl Iterator<Item = (f64, &'a T)>) -> Vec<(f64, String)> {
    options
        .map(|(strike, option)| (strike, option.as_ref().to_owned()))
        .collect()
}

#[test]
fn the_builder_builds_the_chains_of_a_file() {
    let profiles = InstrumentProfileReader::new()
        .unwrap()
        .read_from_file(&fixture())
        .unwrap();
    let chains = OptionChainsBuilder::build(&profiles).into_chains();

    let mut keys: Vec<&str> = chains.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["/ES", "/ESZ26", "AAPL"]);

    let aapl = &chains["AAPL"];
    assert_eq!(aapl.symbol(), "AAPL");
    let series = aapl.series();
    assert_eq!(
        series.len(),
        2,
        "the options without a CFI code or an expiration are skipped"
    );

    let november = &series[0];
    assert_eq!(november.expiration(), NOVEMBER_20_2026);
    assert_eq!(november.strikes(), [200.0]);
    assert_eq!(
        november.to_string(),
        "expiration=20261120, multiplier=100.0, spc=100.0, optionType=SDO, cfi=OXASPS"
    );

    let december = &series[1];
    assert_eq!(december.expiration(), DECEMBER_18_2026);
    assert_eq!(december.last_trade(), DECEMBER_18_2026);
    assert_eq!(december.multiplier(), 100.0);
    assert_eq!(december.spc(), 100.0);
    assert_eq!(december.option_type(), "STAN");
    assert_eq!(december.cfi(), "OXASPS", "calls and puts share a series");
    assert_eq!(
        december.to_string(),
        "expiration=20261218, lastTrade=20261218, multiplier=100.0, spc=100.0, optionType=STAN, cfi=OXASPS"
    );
    assert_eq!(december.strikes(), [190.0, 200.0, 210.0]);
    let calls: Vec<(f64, &str)> = december
        .calls()
        .map(|(strike, ip)| (strike, ip.symbol.as_str()))
        .collect();
    assert_eq!(
        calls,
        [
            (190.0, ".AAPL261218C190"),
            (200.0, ".AAPL261218C200"),
            (210.0, ".AAPL261218C210")
        ]
    );
    let puts: Vec<(f64, &str)> = december
        .puts()
        .map(|(strike, ip)| (strike, ip.symbol.as_str()))
        .collect();
    assert_eq!(puts, [(200.0, ".AAPL261218P200"), (210.0, ".AAPL261218P210")]);
    assert_eq!(
        december.call(190.0).map(|ip| ip.symbol.as_str()),
        Some(".AAPL261218C190")
    );
    assert!(december.put(190.0).is_none());

    // A future option is in the chains of its product and of its underlying.
    for symbol in ["/ES", "/ESZ26"] {
        let series = chains[symbol].series();
        assert_eq!(series.len(), 1);
        assert_eq!(series[0].cfi(), "OXAFPS");
        assert_eq!(
            series[0].call(6000.0).map(|ip| ip.symbol.as_str()),
            Some("./ESZ26C6000")
        );
    }
}

#[test]
fn strikes_around_a_price() {
    let profiles = InstrumentProfileReader::new()
        .unwrap()
        .read_from_file(&fixture())
        .unwrap();
    let chains = OptionChainsBuilder::build(&profiles).into_chains();
    let december = &chains["AAPL"].series()[1];

    assert_eq!(december.n_strikes_around(1, 200.0), [200.0]);
    assert_eq!(december.n_strikes_around(2, 201.0), [200.0, 210.0]);
    assert_eq!(december.n_strikes_around(3, 200.0), [190.0, 200.0, 210.0]);
    assert_eq!(december.n_strikes_around(5, 100.0), [190.0, 200.0, 210.0]);
    assert_eq!(december.n_strikes_around(5, 500.0), [200.0, 210.0]);
    assert!(december.n_strikes_around(0, 200.0).is_empty());
}

#[test]
fn the_builder_takes_any_options() {
    let mut builder = OptionChainsBuilder::<String>::new();
    builder.set_underlying("IBM");
    builder.set_expiration(DECEMBER_18_2026);
    builder.set_strike(100.0);
    builder.set_cfi("OCXXXX");
    builder.add_option("first call".to_owned());
    // A second option of the same kind and strike replaces the first one, as in Java.
    builder.add_option("second call".to_owned());
    builder.set_cfi("OPXXXX");
    builder.add_option("put".to_owned());
    builder.set_cfi("ESXXXX");
    builder.add_option("not an option".to_owned());
    builder.set_cfi("OCXXXX");
    builder.set_strike(f64::NAN);
    builder.add_option("no strike".to_owned());

    let chains = builder.chains();
    assert_eq!(chains.len(), 1);
    let series = chains["IBM"].series();
    assert_eq!(series.len(), 1);
    assert_eq!(series[0].cfi(), "OXXXXX");
    assert_eq!(symbols(series[0].calls()), [(100.0, "second call".to_owned())]);
    assert_eq!(symbols(series[0].puts()), [(100.0, "put".to_owned())]);
    assert_eq!(series[0].to_string(), "expiration=20261218, cfi=OXXXXX");
}

#[test]
fn series_are_compared_by_their_attributes() {
    let mut builder = OptionChainsBuilder::<i32>::new();
    builder.set_underlying("IBM");
    builder.set_cfi("OCXXXX");
    builder.set_strike(1.0);
    for (expiration, spc) in [
        (DECEMBER_18_2026, 100.0),
        (NOVEMBER_20_2026, 100.0),
        (DECEMBER_18_2026, 10.0),
    ] {
        builder.set_expiration(expiration);
        builder.set_spc(spc);
        builder.add_option(expiration);
    }
    let series: &[OptionSeries<i32>] = builder.chains()["IBM"].series();
    let order: Vec<(i32, f64)> = series.iter().map(|s| (s.expiration(), s.spc())).collect();
    assert_eq!(
        order,
        [
            (NOVEMBER_20_2026, 100.0),
            (DECEMBER_18_2026, 10.0),
            (DECEMBER_18_2026, 100.0)
        ]
    );
    assert!(series[0] < series[1]);
    assert_ne!(series[1], series[2]);
}

#[test]
fn options_without_a_kind_or_a_finite_strike_are_skipped() {
    let mut builder = OptionChainsBuilder::<i32>::new();
    builder.set_underlying("IBM");
    builder.set_expiration(DECEMBER_18_2026);
    builder.set_cfi("OCXXXX");
    for strike in [f64::INFINITY, f64::NEG_INFINITY] {
        builder.set_strike(strike);
        builder.add_option(1);
    }
    builder.set_strike(100.0);
    for cfi in ["", "O"] {
        builder.set_cfi(cfi);
        builder.add_option(2);
    }
    assert!(builder.chains().is_empty());
}

#[test]
fn a_series_prints_all_its_attributes() {
    let mut builder = OptionChainsBuilder::<i32>::new();
    builder.set_product("/ES");
    builder.set_expiration(DECEMBER_18_2026);
    builder.set_last_trade(NOVEMBER_20_2026);
    builder.set_multiplier(1e7);
    builder.set_spc(0.5);
    builder.set_additional_underlyings("SE=50; FIS=53");
    builder.set_mmy("202612");
    builder.set_option_type("STAN");
    builder.set_expiration_style("Quarterlys");
    builder.set_settlement_style("Close");
    builder.set_cfi("OPEFPS");
    builder.set_strike(10.0);
    builder.add_option(1);
    assert_eq!(
        builder.chains()["/ES"].series()[0].to_string(),
        concat!(
            "expiration=20261218, lastTrade=20261120, multiplier=1.0E7, spc=0.5, ",
            "additionalUnderlyings=SE=50; FIS=53, mmy=202612, optionType=STAN, expirationStyle=Quarterlys, ",
            "settlementStyle=Close, cfi=OXEFPS"
        )
    );
}

#[test]
fn equal_series_ignore_their_options() {
    let build = |symbol: &str, strike: f64| {
        let mut builder = OptionChainsBuilder::<i32>::new();
        builder.set_underlying(symbol);
        builder.set_expiration(DECEMBER_18_2026);
        builder.set_cfi("OCXXXX");
        builder.set_strike(strike);
        builder.add_option(1);
        builder.into_chains().remove(symbol).unwrap().series()[0].clone()
    };
    let (first, second) = (build("IBM", 100.0), build("MSFT", 200.0));
    assert_eq!(first, second);
    assert_eq!(HashSet::from([first, second]).len(), 1);
}

#[test]
fn a_product_equal_to_the_underlying_takes_an_option_once() {
    let mut builder = OptionChainsBuilder::<i32>::new();
    builder.set_product("IBM");
    builder.set_underlying("IBM");
    builder.set_expiration(DECEMBER_18_2026);
    builder.set_cfi("OCXXXX");
    builder.set_strike(100.0);
    builder.add_option(1);
    let series = builder.chains()["IBM"].series();
    assert_eq!(series.len(), 1);
    assert_eq!(series[0].calls().len(), 1);
}
