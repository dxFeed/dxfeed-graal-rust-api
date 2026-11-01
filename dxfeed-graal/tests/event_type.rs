//! Event type names: no SDK needed.

use std::collections::HashSet;

use dxfeed_graal::{Error, EventType};

#[test]
fn every_event_type_parses_back_from_its_name() {
    for &event_type in EventType::ALL {
        assert_eq!(event_type.get_name().parse::<EventType>().unwrap(), event_type);
        assert_eq!(event_type.to_string(), event_type.get_name());
    }
}

#[test]
fn all_lists_each_of_the_27_types_once() {
    let unique: HashSet<EventType> = EventType::ALL.iter().copied().collect();
    assert_eq!(EventType::ALL.len(), 27);
    assert_eq!(unique.len(), 27);
}

#[test]
fn names_are_the_java_class_names() {
    assert_eq!(EventType::Quote.get_name(), "Quote");
    assert_eq!(EventType::TimeAndSale.get_name(), "TimeAndSale");
    assert_eq!(EventType::TradeETH.get_name(), "TradeETH");
}

#[test]
fn other_strings_do_not_parse() {
    for name in ["quote", "QUOTE", "", " Quote", "Quotes"] {
        assert!(
            matches!(name.parse::<EventType>(), Err(Error::InvalidArgument(_))),
            "{name:?}"
        );
    }
}
