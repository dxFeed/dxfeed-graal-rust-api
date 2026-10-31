//! The attributes of candle symbols as Java has them: parsing, the attributes of a symbol, their normalization and
//! equality. Offline, no network needed.

use std::collections::HashSet;

use dxfeed_graal::Error;
use dxfeed_graal::event::{
    Candle, CandleAlignment, CandleDataType, CandleExchange, CandlePeriod, CandlePrice, CandlePriceLevel,
    CandleSession, CandleType, DailyCandle, Series, Underlying,
};
use dxfeed_graal::schedule::SessionFilter;

#[test]
fn values_parse_as_the_java_documentation_says() {
    assert_eq!(CandleSession::parse("f").unwrap(), CandleSession::Any);
    assert_eq!(CandleSession::parse("TRUE").unwrap(), CandleSession::Regular);
    assert_eq!(CandleType::parse("Minutes").unwrap(), CandleType::Minute);
    assert_eq!(CandleType::parse("pRiCe_M").unwrap(), CandleType::PriceMomentum);
    assert!(matches!(
        CandlePrice::parse("SETTLE"),
        Err(Error::InvalidArgument(message)) if message == "Unknown candle price: SETTLE"
    ));
    assert_eq!("s".parse::<CandlePrice>().unwrap(), CandlePrice::Settlement);
    assert_eq!(CandleAlignment::parse("S").unwrap(), CandleAlignment::Session);
    assert_eq!(CandleType::ALL.len(), 13);
    assert_eq!(
        CandlePrice::ALL,
        [
            CandlePrice::Last,
            CandlePrice::Bid,
            CandlePrice::Ask,
            CandlePrice::Mark,
            CandlePrice::Settlement
        ]
    );
}

#[test]
fn price_levels_are_never_negative_nor_infinite() {
    for value in [-1.0, -0.0, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(
            matches!(CandlePriceLevel::value_of(value), Err(Error::InvalidArgument(_))),
            "{value}"
        );
    }
    assert!(matches!(
        CandlePriceLevel::value_of(-1.0),
        Err(Error::InvalidArgument(message)) if message == "Incorrect candle price level: -1.0"
    ));
    assert_eq!(CandlePriceLevel::value_of(f64::NAN).unwrap(), CandlePriceLevel::DEFAULT);
    assert_eq!(CandlePriceLevel::parse("0.5").unwrap().get_value(), 0.5);
    assert_eq!(CandlePriceLevel::parse("1E1").unwrap().to_string(), "10");
    assert!(CandlePriceLevel::parse("-1").is_err());
    assert!(matches!(
        CandlePriceLevel::parse(" zzz "),
        Err(Error::InvalidArgument(message)) if message == "For input string: \"zzz\""
    ));
}

#[test]
fn symbols_give_their_attributes() {
    assert_eq!(
        CandleDataType::get_attribute_for_symbol("AAPL{dt=tho}"),
        CandleDataType::THO
    );
    assert_eq!(
        CandleDataType::get_attribute_for_symbol("AAPL{dt=xyz}").get_value(),
        "xyz"
    );
    assert_eq!(
        CandleDataType::get_attribute_for_symbol("AAPL"),
        CandleDataType::DEFAULT
    );
    assert_eq!(
        CandleExchange::get_attribute_for_symbol("AAPL&Q{=d}"),
        CandleExchange::value_of('Q')
    );
    assert_eq!(
        CandleExchange::get_attribute_for_symbol("AAPL{=d}"),
        CandleExchange::COMPOSITE
    );
    assert_eq!(CandleExchange::COMPOSITE.to_string(), "COMPOSITE");
    assert_eq!(
        CandlePrice::get_attribute_for_symbol("AAPL{price=bid}").unwrap(),
        CandlePrice::Bid
    );
    assert_eq!(
        CandlePrice::get_attribute_for_symbol("AAPL").unwrap(),
        CandlePrice::DEFAULT
    );
    assert!(CandlePrice::get_attribute_for_symbol("AAPL{price=zzz}").is_err());
    assert_eq!(
        CandleSession::get_attribute_for_symbol("AAPL{tho=TRUE}"),
        CandleSession::Regular
    );
    assert_eq!(
        CandleSession::get_attribute_for_symbol("AAPL{tho=t}"),
        CandleSession::Any
    );
    assert_eq!(
        CandleAlignment::get_attribute_for_symbol("AAPL{a=S}").unwrap(),
        CandleAlignment::Session
    );
    assert_eq!(
        CandlePriceLevel::get_attribute_for_symbol("AAPL{pl=2}")
            .unwrap()
            .get_value(),
        2.0
    );
    assert_eq!(
        CandlePriceLevel::get_attribute_for_symbol("AAPL").unwrap(),
        CandlePriceLevel::DEFAULT
    );

    assert_eq!(
        [
            CandlePeriod::ATTRIBUTE_KEY,
            CandlePrice::ATTRIBUTE_KEY,
            CandleSession::ATTRIBUTE_KEY,
            CandleAlignment::ATTRIBUTE_KEY,
            CandlePriceLevel::ATTRIBUTE_KEY,
            CandleDataType::ATTRIBUTE_KEY
        ],
        ["", "price", "tho", "a", "pl", "dt"]
    );
    assert_eq!(CandleSession::Any.get_session_filter(), SessionFilter::Any);
    assert_eq!(CandleSession::Regular.get_session_filter(), SessionFilter::Regular);
}

#[test]
fn attributes_normalize_as_in_java() {
    assert_eq!(
        CandlePrice::normalize_attribute_for_symbol("AAPL{price=BI}"),
        "AAPL{price=bid}"
    );
    // Java drops the result of removing a default value, so the value stays.
    assert_eq!(
        CandlePrice::normalize_attribute_for_symbol("AAPL{price=last}"),
        "AAPL{price=last}"
    );
    assert_eq!(
        CandlePrice::normalize_attribute_for_symbol("AAPL{price=zzz}"),
        "AAPL{price=zzz}"
    );
    assert_eq!(
        CandleSession::normalize_attribute_for_symbol("AAPL{tho=TRUE}"),
        "AAPL{tho=true}"
    );
    assert_eq!(
        CandleSession::normalize_attribute_for_symbol("AAPL{tho=false}"),
        "AAPL{tho=false}"
    );
    assert_eq!(
        CandleAlignment::normalize_attribute_for_symbol("AAPL{a=S}"),
        "AAPL{a=s}"
    );
    assert_eq!(
        CandlePriceLevel::normalize_attribute_for_symbol("AAPL{pl=1.0}"),
        "AAPL{pl=1}"
    );
    assert_eq!(
        CandleDataType::normalize_attribute_for_symbol("AAPL{=d,dt=}"),
        "AAPL{=d}"
    );
    assert_eq!(
        CandleDataType::normalize_attribute_for_symbol("AAPL{dt=tho}"),
        "AAPL{dt=tho}"
    );
}

#[test]
fn periods_and_levels_compare_as_java_doubles() {
    let nan = CandlePeriod::value_of(f64::NAN, CandleType::Minute);
    assert_eq!(nan, nan);
    assert_ne!(
        CandlePeriod::value_of(0.0, CandleType::Minute),
        CandlePeriod::value_of(-0.0, CandleType::Minute)
    );
    assert_eq!(CandlePeriod::DEFAULT, CandlePeriod::TICK);
    let periods: HashSet<_> = [
        CandlePeriod::value_of(1.0, CandleType::Day),
        CandlePeriod::DAY,
        nan,
        CandlePeriod::value_of(-f64::NAN, CandleType::Minute),
    ]
    .into_iter()
    .collect();
    assert_eq!(periods.len(), 2);
    assert_eq!(CandlePeriod::value_of(1e19, CandleType::Volume).to_string(), "1.0E19v");
    assert_eq!(CandlePeriod::value_of(-0.0, CandleType::Volume).to_string(), "0v");
}

#[test]
fn periods_parse_as_in_java() {
    assert_eq!(
        CandlePeriod::parse("5m").unwrap(),
        CandlePeriod::value_of(5.0, CandleType::Minute)
    );
    assert_eq!(CandlePeriod::parse("d").unwrap(), CandlePeriod::DAY);
    assert_eq!(CandlePeriod::parse("Days").unwrap(), CandlePeriod::DAY);
    assert_eq!(
        CandlePeriod::parse("1.5h").unwrap(),
        CandlePeriod::value_of(1.5, CandleType::Hour)
    );
    assert_eq!(
        "2E1s".parse::<CandlePeriod>().unwrap(),
        CandlePeriod::value_of(20.0, CandleType::Second)
    );
    assert!(matches!(
        CandlePeriod::parse("1e"),
        Err(Error::InvalidArgument(message)) if message == "For input string: \"1e\""
    ));
    assert!(matches!(
        CandlePeriod::parse("1-m"),
        Err(Error::InvalidArgument(message)) if message == "For input string: \"1-\""
    ));
    assert!(matches!(
        CandlePeriod::parse("5"),
        Err(Error::InvalidArgument(message)) if message == "Missing candle type"
    ));
    assert!(matches!(
        CandlePeriod::parse("5x"),
        Err(Error::InvalidArgument(message)) if message == "Unknown candle type: x"
    ));
    assert_eq!(
        CandlePeriod::get_attribute_for_symbol("AAPL{=5m}").unwrap().to_string(),
        "5m"
    );
    assert_eq!(
        CandlePeriod::get_attribute_for_symbol("AAPL{=d,=h}").unwrap(),
        CandlePeriod::DAY
    );
    assert_eq!(
        CandlePeriod::get_attribute_for_symbol("AAPL").unwrap(),
        CandlePeriod::DEFAULT
    );
    assert_eq!(CandlePeriod::normalize_attribute_for_symbol("AAPL{=1d}"), "AAPL{=d}");
    assert_eq!(CandlePeriod::normalize_attribute_for_symbol("AAPL{=1t}"), "AAPL{=t}");
    assert_eq!(CandlePeriod::normalize_attribute_for_symbol("AAPL{=zzz}"), "AAPL{=zzz}");
}

#[test]
fn periods_give_their_width() {
    assert_eq!(
        CandlePeriod::value_of(5.0, CandleType::Minute).get_period_interval_millis(),
        300_000
    );
    assert_eq!(
        CandlePeriod::value_of(1.5, CandleType::Second).get_period_interval_millis(),
        1500
    );
    assert_eq!(
        CandlePeriod::value_of(1.0, CandleType::Year).get_period_interval_millis(),
        31_536_000_000
    );
    assert_eq!(
        CandlePeriod::value_of(100.0, CandleType::Volume).get_period_interval_millis(),
        0
    );
    assert_eq!(
        CandlePeriod::value_of(f64::NAN, CandleType::Day).get_period_interval_millis(),
        0
    );
    assert_eq!(
        CandlePeriod::value_of(1e300, CandleType::Day).get_period_interval_millis(),
        i64::MAX
    );
    assert_eq!(CandleType::Month.get_period_interval_millis(), 2_592_000_000);
    assert_eq!(CandleType::Tick.get_period_interval_millis(), 0);
}

#[test]
fn daily_candles_convert_to_candles() {
    let mut daily = DailyCandle::new("AAPL{=d}");
    daily.close = 10.5;
    daily.count = 3;
    let candle = Candle::from(daily.clone());
    assert_eq!(
        (candle.symbol.as_str(), candle.close, candle.count),
        ("AAPL{=d}", 10.5, 3)
    );
    // The NaN prices make the events unequal, so their texts are compared.
    assert_eq!(DailyCandle::from(candle).to_string(), daily.to_string());
}

#[test]
fn option_volume_skips_unknown_volumes() {
    let mut series = Series::new("AAPL");
    assert!(series.option_volume().is_nan());
    series.put_volume = 2.0;
    assert_eq!(series.option_volume(), 2.0);
    series.call_volume = 3.0;
    assert_eq!(series.option_volume(), 5.0);
    let mut underlying = Underlying::new("AAPL");
    underlying.call_volume = 7.0;
    assert_eq!(underlying.option_volume(), 7.0);
}
