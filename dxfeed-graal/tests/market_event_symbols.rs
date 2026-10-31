//! `market_event_symbols` gives the strings of Java's `MarketEventSymbols`: every expected value below is the output
//! of the Java class for the same input.

use dxfeed_graal::Error;
use dxfeed_graal::event::market_event_symbols::{
    DEFAULT_EXCHANGES, SUPPORTED_EXCHANGES, build_spread_symbol, change_attribute_string_by_key, change_base_symbol,
    change_exchange_code, get_attribute_string_by_key, get_base_symbol, get_exchange_code, get_exchanges_by_pattern,
    has_attributes, has_exchange_code, remove_attribute_string_by_key,
};

#[test]
fn exchange_codes_follow_the_ampersand() {
    assert!(!has_exchange_code("AAPL"));
    assert_eq!(get_exchange_code("AAPL"), '\0');
    assert_eq!(get_exchange_code("AAPL&Q"), 'Q');
    assert_eq!(get_exchange_code("AAPL&Q{=d}"), 'Q');
    assert_eq!(get_exchange_code("é&é{k=v}"), 'é');
    assert!(!has_exchange_code("AAPL&{=d}"));
    assert!(has_exchange_code("&Q"));

    assert_eq!(change_exchange_code("AAPL", 'X'), "AAPL&X");
    assert_eq!(change_exchange_code("AAPL&Q{=d}", 'X'), "AAPL&X{=d}");
    assert_eq!(change_exchange_code("AAPL&Q{=d}", '\0'), "AAPL{=d}");
    assert_eq!(change_exchange_code("&Q", '\0'), "");
    assert_eq!(change_exchange_code("AAPL&{=d}", 'X'), "AAPL&&X{=d}");
    assert_eq!(change_exchange_code("A{}", 'X'), "A{}&X");
}

#[test]
fn base_symbols_drop_the_exchange_code_and_the_attributes() {
    assert_eq!(get_base_symbol("IBM&N{a=1,c=3}"), "IBM");
    assert_eq!(get_base_symbol("{x}"), "");
    assert_eq!(get_base_symbol("AAPL&Q{"), "AAPL&Q{");
    assert_eq!(change_base_symbol("IBM&N{a=1,c=3}", "MSFT"), "MSFT&N{a=1,c=3}");
    assert_eq!(change_base_symbol("AAPL&Q{", "MSFT"), "MSFT");
    assert_eq!(change_base_symbol("AAPL{}x}", "MSFT"), "MSFT{}x}");
}

#[test]
fn attributes_need_a_non_empty_closing_brace() {
    assert!(has_attributes("AAPL{=d,price=bid}"));
    assert!(has_attributes("{x}"));
    assert!(!has_attributes("A{}"));
    assert!(!has_attributes("AAPL&Q{"));
}

#[test]
fn attributes_are_read_by_key() {
    assert_eq!(get_attribute_string_by_key("AAPL{=d,price=bid}", ""), Some("d"));
    assert_eq!(get_attribute_string_by_key("AAPL{=d,price=bid}", "price"), Some("bid"));
    assert_eq!(get_attribute_string_by_key("AAPL{=d,price=bid}", "a"), None);
    assert_eq!(get_attribute_string_by_key("AAPL{c=3,a=1,c=4}", "c"), Some("3"));
    assert_eq!(get_attribute_string_by_key("AAPL{a,b=2}", "b"), None);
    assert_eq!(get_attribute_string_by_key("AAPL", "price"), None);
}

#[test]
fn attributes_are_set_in_key_order() {
    let set = |symbol, key| change_attribute_string_by_key(symbol, key, Some("v"));
    assert_eq!(set("AAPL", "price"), "AAPL{price=v}");
    assert_eq!(set("AAPL{=d}", "price"), "AAPL{=d,price=v}");
    assert_eq!(set("AAPL{=d,price=bid}", ""), "AAPL{=v,price=bid}");
    assert_eq!(set("AAPL{=d,price=bid}", "a"), "AAPL{=d,a=v,price=bid}");
    assert_eq!(set("AAPL{=d,price=bid}", "z"), "AAPL{=d,price=bid,z=v}");
    assert_eq!(set("AAPL&Q{a=1,c=3}", "b"), "AAPL&Q{a=1,b=v,c=3}");
    assert_eq!(set("AAPL{c=3,a=1,c=4}", "c"), "AAPL{c=v,a=1}");
    assert_eq!(set("AAPL{c=3,a=1,c=4}", "a"), "AAPL{a=v,c=3,c=4}");
    assert_eq!(set("AAPL{x}", "a"), "AAPL,a=v{x}");
    assert_eq!(set("AAPL{a=1,b}", "c"), "AAPL{a=1,c=v,b}");
}

#[test]
fn removing_the_last_attribute_removes_the_braces() {
    assert_eq!(change_attribute_string_by_key("AAPL{price=bid}", "price", None), "AAPL");
    assert_eq!(remove_attribute_string_by_key("AAPL{price=bid}", "price"), "AAPL");
    assert_eq!(
        remove_attribute_string_by_key("AAPL{=d,price=bid}", ""),
        "AAPL{price=bid}"
    );
    assert_eq!(
        remove_attribute_string_by_key("AAPL{=d,price=bid}", "price"),
        "AAPL{=d}"
    );
    assert_eq!(remove_attribute_string_by_key("AAPL{c=3,a=1,c=4}", "c"), "AAPL{a=1}");
    assert_eq!(remove_attribute_string_by_key("AAPL{a=1,b}", "a"), "AAPL{b}");
    assert_eq!(change_attribute_string_by_key("AAPL", "price", None), "AAPL");
}

#[test]
fn spread_symbols_sort_their_legs() {
    assert_eq!(build_spread_symbol::<_, &str, f64>([]), "");
    assert_eq!(build_spread_symbol([("AAPL", 1.0)]), "AAPL");
    assert_eq!(build_spread_symbol([("AAPL", 2.0)]), "=2*AAPL");
    assert_eq!(build_spread_symbol([("AAPL", -1.0)]), "=-AAPL");
    let legs = [
        ("IBM", -1.0),
        ("AAPL", 2.0),
        ("MSFT", 0.0),
        ("GOOG", 0.5),
        ("C", -1.0),
        ("B", -1.0),
        ("X", 1.0),
        ("Y", 1e10),
        ("Z", 1.25e-5),
    ];
    let expected = "=1.0E10*Y+2*AAPL+X+0.5*GOOG+1.25E-5*Z-IBM-C-B";
    assert_eq!(build_spread_symbol(legs), expected);
    let map: std::collections::HashMap<String, f64> = legs.iter().map(|&(s, r)| (s.to_owned(), r)).collect();
    assert_eq!(build_spread_symbol(&map), expected);
}

#[test]
fn exchanges_come_from_a_character_class() {
    assert_eq!(get_exchanges_by_pattern("").unwrap(), "");
    assert_eq!(get_exchanges_by_pattern("A-Z").unwrap(), DEFAULT_EXCHANGES);
    assert_eq!(get_exchanges_by_pattern("[A-DQ]").unwrap(), "ABCDQ");
    assert_eq!(get_exchanges_by_pattern("\\d").unwrap(), "0123456789");
    assert_eq!(get_exchanges_by_pattern("[\\dX]").unwrap(), "0123456789X");
    assert_eq!(
        get_exchanges_by_pattern("[A-Z&&[^N]]").unwrap(),
        "ABCDEFGHIJKLMOPQRSTUVWXYZ"
    );
    assert_eq!(get_exchanges_by_pattern("[0-9A-Za-z]").unwrap(), SUPPORTED_EXCHANGES);
}

#[test]
fn non_ascii_symbols_keep_their_characters() {
    let symbol = "Ä&é{ключ=знач,ü=1}";
    assert_eq!(get_base_symbol(symbol), "Ä");
    assert_eq!(get_attribute_string_by_key(symbol, "ü"), Some("1"));
    assert_eq!(get_attribute_string_by_key(symbol, "ключ"), Some("знач"));
    assert_eq!(
        change_attribute_string_by_key(symbol, "ü", Some("ß")),
        "Ä&é{ü=ß,ключ=знач}"
    );
    assert_eq!(
        change_attribute_string_by_key(symbol, "a", Some("ß")),
        "Ä&é{a=ß,ключ=знач,ü=1}"
    );
    assert_eq!(remove_attribute_string_by_key(symbol, "ключ"), "Ä&é{ü=1}");
    assert_eq!(remove_attribute_string_by_key(symbol, "ü"), "Ä&é{ключ=знач}");

    // Java sees two UTF-16 units after `&`, so there is no exchange code.
    assert!(!has_exchange_code("A&😀"));
    assert_eq!(get_base_symbol("A&😀"), "A&😀");
    assert_eq!(change_exchange_code("A&😀", 'X'), "A&😀&X");
}

#[test]
fn intersections_leave_out_an_empty_operand() {
    assert_eq!(get_exchanges_by_pattern("A-Z&&").unwrap(), DEFAULT_EXCHANGES);
    assert_eq!(get_exchanges_by_pattern("&&A-Z").unwrap(), DEFAULT_EXCHANGES);
    assert_eq!(get_exchanges_by_pattern("[A-Z&&]").unwrap(), DEFAULT_EXCHANGES);
    assert_eq!(get_exchanges_by_pattern("A-C&&B-D").unwrap(), "BC");
    assert_eq!(
        get_exchanges_by_pattern("[A-Z&&[^N]&&[^Q]]").unwrap(),
        "ABCDEFGHIJKLMOPRSTUVWXYZ"
    );
}

#[test]
fn exchanges_must_be_digits_or_letters() {
    assert!(
        matches!(get_exchanges_by_pattern("[\\t-~]"), Err(Error::InvalidArgument(message))
        if message.starts_with("Unsupported exchange codes: \t, "))
    );
    let message = |pattern| match get_exchanges_by_pattern(pattern) {
        Err(Error::InvalidArgument(message)) => message,
        other => panic!("{pattern}: {other:?}"),
    };
    assert_eq!(message("\\w"), "Unsupported exchange codes: _");
    assert_eq!(message("!"), "Unsupported exchange codes: !");
    assert!(message("[A-").starts_with("unsupported character class"));
    assert!(message("\\p{Upper}").starts_with("unsupported character class"));
}
