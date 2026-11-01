//! The glossary of instrument attributes: price increments and additional underlyings, computed by Java.

use std::collections::BTreeMap;

use dxfeed_graal::Error;
use dxfeed_graal::glossary::{AdditionalUnderlyings, PriceIncrements, RoundingMode};

#[test]
fn price_increments_compute_as_in_java() {
    let increments = PriceIncrements::value_of("0.01 1; 0.05").unwrap();
    assert_eq!(increments.get_text(), "0.01 1; 0.05");
    assert_eq!(increments.to_string(), "0.01 1; 0.05");
    assert_eq!(increments.get_price_increments().unwrap(), [0.01, 1.0, 0.05]);
    assert_eq!(increments.get_price_increment().unwrap(), 0.01);
    assert_eq!(increments.get_price_increment_for(2.0).unwrap(), 0.05);
    assert_eq!(increments.get_price_increment_for(1.0).unwrap(), 0.01);
    assert_eq!(increments.get_price_increment_in_direction(1.0, 1).unwrap(), 0.05);
    assert_eq!(increments.get_price_precision().unwrap(), 2);
    assert_eq!(increments.get_price_precision_for(2.0).unwrap(), 2);

    assert_eq!(increments.round_price(1.234).unwrap(), 1.25);
    assert_eq!(increments.round_price_in_direction(1.234, -1).unwrap(), 1.2);
    assert_eq!(
        increments.round_price_with_mode(1.26, RoundingMode::Floor).unwrap(),
        1.25
    );
    assert!(matches!(
        increments.round_price_with_mode(1.26, RoundingMode::Unnecessary),
        Err(Error::Java(_))
    ));
    assert!(increments.round_price(f64::NAN).unwrap().is_nan());
    assert_eq!(increments.increment_price(0.99, 1).unwrap(), 1.0);
    assert_eq!(increments.increment_price(1.0, 1).unwrap(), 1.05);
    assert_eq!(increments.increment_price(1.0, -1).unwrap(), 0.99);
    assert_eq!(increments.increment_price_with_step(1.0, 1, 0.1).unwrap(), 1.1);
    assert!(matches!(increments.increment_price(1.0, 0), Err(Error::Java(_))));

    assert_eq!(
        PriceIncrements::value_of_increments(&[0.01, 1.0, 0.05]).unwrap(),
        increments
    );
    assert_eq!("0.01 1; 0.05".parse::<PriceIncrements>().unwrap(), increments);
    let quarter = PriceIncrements::value_of_increment(0.25).unwrap();
    assert_eq!(quarter.get_text(), "0.25");
    assert_eq!(quarter.round_price(1.1).unwrap(), 1.0);

    let empty = PriceIncrements::empty().unwrap();
    assert_eq!(empty.get_text(), "");
    assert_eq!(empty.get_price_increments().unwrap(), [0.0]);
    assert_eq!(empty.round_price(1.234).unwrap(), 1.234);
    assert_eq!(PriceIncrements::value_of("").unwrap(), empty);
    assert_eq!(PriceIncrements::value_of_increment(0.0).unwrap(), empty);

    assert!(matches!(PriceIncrements::value_of("x"), Err(Error::Java(_))));
    assert!(matches!(
        PriceIncrements::value_of("\0"),
        Err(Error::InvalidArgument(_))
    ));
    assert!(matches!(PriceIncrements::value_of_increment(-1.0), Err(Error::Java(_))));
    assert!(matches!(
        PriceIncrements::value_of_increments(&[0.01, 1.0]),
        Err(Error::Java(_))
    ));
}

#[test]
fn rounding_modes_have_the_java_names() {
    let names: Vec<_> = RoundingMode::ALL.iter().map(|mode| mode.to_string()).collect();
    assert_eq!(
        names,
        [
            "UP",
            "DOWN",
            "CEILING",
            "FLOOR",
            "HALF_UP",
            "HALF_DOWN",
            "HALF_EVEN",
            "UNNECESSARY"
        ]
    );
    assert_eq!("HALF_EVEN".parse::<RoundingMode>().unwrap(), RoundingMode::HalfEven);
    assert!(matches!(
        "half_even".parse::<RoundingMode>(),
        Err(Error::InvalidArgument(_))
    ));
    let increments = PriceIncrements::value_of("0.5").unwrap();
    let rounded: Vec<_> = RoundingMode::ALL[..7]
        .iter()
        .map(|&mode| increments.round_price_with_mode(-1.25, mode).unwrap())
        .collect();
    assert_eq!(rounded, [-1.5, -1.0, -1.0, -1.5, -1.5, -1.0, -1.0]);
}

#[test]
fn additional_underlyings_read_as_in_java() {
    let underlyings = AdditionalUnderlyings::value_of("SE 50; FIS 53").unwrap();
    assert_eq!(underlyings.get_text(), "SE 50; FIS 53");
    assert_eq!(underlyings.to_string(), "SE 50; FIS 53");
    assert_eq!(underlyings.get_spc("SE").unwrap(), 50.0);
    assert_eq!(underlyings.get_spc("IBM").unwrap(), 0.0);
    let map = underlyings.get_map().unwrap();
    assert_eq!(map, BTreeMap::from([("FIS".to_owned(), 53.0), ("SE".to_owned(), 50.0)]));
    assert_eq!(
        AdditionalUnderlyings::get_spc_from_text("SE 50; FIS 53", "FIS").unwrap(),
        53.0
    );

    let made = AdditionalUnderlyings::value_of_map(map.clone()).unwrap();
    assert_eq!(made.get_text(), "FIS 53; SE 50");
    assert_ne!(made, underlyings);
    assert_eq!(made.get_map().unwrap(), map);
    assert_eq!(
        AdditionalUnderlyings::value_of_map([("SE", 50.0), ("FIS", 53.0), ("SE", 50.0)]).unwrap(),
        made
    );
    assert_eq!("FIS 53; SE 50".parse::<AdditionalUnderlyings>().unwrap(), made);

    let empty = AdditionalUnderlyings::empty().unwrap();
    assert_eq!(empty.get_text(), "");
    assert!(empty.get_map().unwrap().is_empty());
    assert_eq!(AdditionalUnderlyings::value_of("").unwrap(), empty);
    assert_eq!(
        AdditionalUnderlyings::value_of_map(Vec::<(&str, f64)>::new()).unwrap(),
        empty
    );

    assert!(matches!(AdditionalUnderlyings::value_of("SE"), Err(Error::Java(_))));
    assert!(matches!(
        AdditionalUnderlyings::value_of_map([("A B", 1.0)]),
        Err(Error::Java(_))
    ));
    assert!(matches!(
        AdditionalUnderlyings::value_of_map([("A\0", 1.0)]),
        Err(Error::InvalidArgument(_))
    ));
    assert_eq!(AdditionalUnderlyings::get_spc_from_text("SE", "SE").unwrap(), 0.0);
    assert_eq!(
        AdditionalUnderlyings::get_spc_from_text("SE x; FIS 53", "FIS").unwrap(),
        53.0
    );
}
