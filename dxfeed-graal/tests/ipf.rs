//! Instrument profiles: reading a file, fields by name, the collector and a connection to a file; no network needed.

use std::cmp::Ordering;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use dxfeed_graal::ipf::InstrumentProfileReader;
use dxfeed_graal::ipf::live::{ConnectionState, InstrumentProfileCollector, InstrumentProfileConnection};
use dxfeed_graal::ipf::{InstrumentProfile, InstrumentProfileField, InstrumentProfileType};
use dxfeed_graal::{AuthToken, Error};

fn fixture() -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/profiles.ipf")
        .to_string_lossy()
        .into_owned()
}

/// The day id of 2026-12-18.
const DECEMBER_18_2026: i32 = 20805;

fn expected() -> Vec<InstrumentProfile> {
    let mut stock = InstrumentProfile {
        instrument_type: "STOCK".into(),
        symbol: "AAPL".into(),
        description: "Apple Inc.".into(),
        country: "US".into(),
        currency: "USD".into(),
        icb: 9572,
        multiplier: 1.0,
        ..InstrumentProfile::new()
    };
    stock.custom_fields.insert("CUSTOM_NOTE".into(), "first".into());
    let option = InstrumentProfile {
        instrument_type: "OPTION".into(),
        symbol: ".AAPL261218C200".into(),
        description: "AAPL Dec 2026 200 Call".into(),
        underlying: "AAPL".into(),
        spc: 100.0,
        multiplier: 100.0,
        expiration: DECEMBER_18_2026,
        strike: 200.5,
        option_type: "STAN".into(),
        ..InstrumentProfile::new()
    };
    vec![stock, option]
}

#[test]
fn the_reader_reads_a_file() {
    let reader = InstrumentProfileReader::new().unwrap();
    let profiles = reader.read_from_file(&fixture()).unwrap();
    assert_eq!(profiles, expected());
    assert!(reader.was_complete().unwrap());
    assert!(reader.get_last_modified().unwrap() > 0);
    assert_eq!(profiles[0].to_string(), "STOCK AAPL");
    // A file path stays as it is.
    assert_eq!(
        InstrumentProfileReader::resolve_source_url(&fixture()).unwrap(),
        fixture()
    );
    assert_eq!(
        InstrumentProfileReader::resolve_source_url("example.com:7777").unwrap(),
        "http://example.com:7777/ipf/all.ipf.gz"
    );
    assert!(matches!(reader.read_from_file("no-such-file.ipf"), Err(Error::Java(_))));
}

#[test]
fn fields_by_name() {
    let mut profile = expected().remove(1);
    assert_eq!(profile.get_field("SYMBOL").unwrap(), ".AAPL261218C200");
    assert_eq!(profile.get_field("STRIKE").unwrap(), "200.5");
    assert_eq!(profile.get_field("SPC").unwrap(), "100");
    assert_eq!(profile.get_field("ICB").unwrap(), "");
    assert_eq!(profile.get_field("EXPIRATION").unwrap(), "2026-12-18");
    assert_eq!(profile.get_field("NO_SUCH_FIELD").unwrap(), "");

    profile.set_field("LAST_TRADE", "2026-12-17").unwrap();
    assert_eq!(profile.last_trade, DECEMBER_18_2026 - 1);
    profile.set_field("SIC", "1234").unwrap();
    assert_eq!(profile.sic, 1234);
    profile.set_field("NOTE", "x").unwrap();
    assert_eq!(profile.get_field("NOTE").unwrap(), "x");
    profile.set_field("NOTE", "").unwrap();
    assert!(profile.custom_fields.is_empty());
    assert!(profile.set_field("STRIKE", "abc").is_err());
    assert_eq!(InstrumentProfile::STANDARD_FIELDS.len(), 31);
}

#[test]
fn the_collector_keeps_and_tells_the_updates() {
    let collector = InstrumentProfileCollector::new().unwrap();
    collector.update_instrument_profiles(&expected()).unwrap();
    let mut view = collector.view().unwrap();
    // `.AAPL261218C200` sorts before `AAPL`.
    view.sort_by(|a, b| b.symbol.cmp(&a.symbol));
    assert_eq!(view, expected());
    assert!(collector.get_last_update_time().unwrap() > 0);

    let (sender, updates) = mpsc::channel();
    let id = collector
        .add_update_listener(move |profiles| {
            let _ = sender.send(profiles.to_vec());
        })
        .unwrap();
    // A listener is told every profile first.
    let first = updates.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(first.len(), 2);

    let mut stock = expected().remove(0);
    stock.description = "Apple".into();
    collector.update_instrument_profile(&stock).unwrap();
    assert_eq!(updates.recv_timeout(Duration::from_secs(10)).unwrap(), [stock.clone()]);

    let removed = InstrumentProfile {
        instrument_type: InstrumentProfile::REMOVED.into(),
        symbol: "AAPL".into(),
        ..InstrumentProfile::new()
    };
    collector.update_instrument_profile(&removed).unwrap();
    let update = updates.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(update[0].is_removed());

    collector.remove_update_listener(id).unwrap();
    assert!(matches!(
        collector.remove_update_listener(id),
        Err(Error::InvalidArgument(_))
    ));
    collector.update_instrument_profile(&stock).unwrap();
    assert!(updates.recv_timeout(Duration::from_millis(300)).is_err());
}

#[test]
fn a_connection_reads_a_file() {
    let collector = InstrumentProfileCollector::new().unwrap();
    let connection = InstrumentProfileConnection::create_connection(&fixture(), &collector).unwrap();
    assert_eq!(connection.get_address().unwrap(), fixture());
    assert_eq!(connection.get_state().unwrap(), ConnectionState::NotConnected);
    connection.set_update_period(Duration::from_secs(30)).unwrap();
    assert_eq!(connection.get_update_period().unwrap(), Duration::from_secs(30));
    let (sender, states) = mpsc::channel();
    connection
        .add_state_change_listener(move |_, state| {
            let _ = sender.send(state);
        })
        .unwrap();

    connection.start().unwrap();
    assert!(connection.wait_until_completed(Duration::from_secs(10)).unwrap());
    assert_eq!(collector.view().unwrap().len(), 2);
    // The wait can return before the state is updated: the listener tells when it is.
    let next = || states.recv_timeout(Duration::from_secs(10)).unwrap();
    let seen: Vec<ConnectionState> = std::iter::from_fn(|| Some(next()))
        .take_while(|state| *state != ConnectionState::Completed)
        .collect();
    assert_eq!(seen, [ConnectionState::Connecting, ConnectionState::Connected]);
    assert_eq!(connection.get_state().unwrap(), ConnectionState::Completed);
    // Java records the modification time just after it reports COMPLETED.
    let deadline = Instant::now() + Duration::from_secs(10);
    while connection.get_last_modified().unwrap() == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(connection.get_last_modified().unwrap() > 0);
    assert!(connection.start().is_err());

    connection.close().unwrap();
    assert_eq!(next(), ConnectionState::Closed);
    assert_eq!(ConnectionState::NotConnected.to_string(), "NOT_CONNECTED");
}

fn data(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data").join(name)
}

#[test]
fn the_reader_reads_bytes() {
    let reader = InstrumentProfileReader::new().unwrap();
    let plain = std::fs::read(data("profiles.ipf")).unwrap();
    assert_eq!(reader.read_from_bytes(&plain, "profiles.ipf").unwrap(), expected());
    assert!(reader.was_complete().unwrap());
    let gzipped = std::fs::read(data("profiles.ipf.gz")).unwrap();
    assert_eq!(reader.read_from_bytes(&gzipped, "profiles.ipf.gz").unwrap(), expected());
    let file = data("profiles.ipf.gz").to_string_lossy().into_owned();
    assert_eq!(reader.read_from_file(&file).unwrap(), expected());
    assert_eq!(reader.read_from_bytes(b"", "empty.ipf").unwrap(), vec![]);
    assert!(!reader.was_complete().unwrap());
    assert!(matches!(
        reader.read_from_bytes(b"", "a\0"),
        Err(Error::InvalidArgument(_))
    ));
}

#[test]
fn the_reader_sends_the_token() {
    let tokens = [
        AuthToken::create_bearer_token("secret-token").unwrap(),
        AuthToken::create_basic_token("user", "pass").unwrap(),
        AuthToken::create_custom_token("Basic", "not-base64").unwrap(),
        AuthToken::create_custom_token("Custom", "a b").unwrap(),
    ];
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://127.0.0.1:{}/profiles.ipf", server.local_addr().unwrap().port());
    let serve = thread::spawn(move || {
        let mut requests = Vec::new();
        for _ in 0..4 {
            let (mut stream, _) = server.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            let body = std::fs::read(data("profiles.ipf")).unwrap();
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(head.as_bytes()).unwrap();
            stream.write_all(&body).unwrap();
            requests.push(String::from_utf8(request).unwrap());
        }
        requests
    });
    let reader = InstrumentProfileReader::new().unwrap();
    for token in &tokens {
        assert_eq!(reader.read_from_file_with_token(&address, token).unwrap(), expected());
    }
    let requests = serve.join().unwrap();
    for (token, request) in tokens.iter().zip(&requests) {
        let header = format!("authorization: {}\r\n", token.get_http_authorization());
        assert!(
            request.to_ascii_lowercase().contains(&header.to_ascii_lowercase()),
            "{request}"
        );
    }
}

#[test]
fn fields_are_enumerated_as_in_java() {
    assert_eq!(InstrumentProfileField::ALL.len(), 31);
    let names: Vec<_> = InstrumentProfileField::ALL.iter().map(|f| f.to_string()).collect();
    assert_eq!(names, InstrumentProfile::STANDARD_FIELDS);
    assert_eq!(
        InstrumentProfileField::find("LOCAL_SYMBOL"),
        Some(InstrumentProfileField::LocalSymbol)
    );
    assert_eq!(InstrumentProfileField::find("local_symbol"), None);
    assert_eq!(
        "LAST_TRADE".parse::<InstrumentProfileField>().unwrap(),
        InstrumentProfileField::LastTrade
    );
    assert!(matches!(
        "NOPE".parse::<InstrumentProfileField>(),
        Err(Error::InvalidArgument(_))
    ));
    let numeric: Vec<_> = InstrumentProfileField::ALL
        .iter()
        .filter(|f| f.is_numeric_field())
        .map(|f| f.get_name())
        .collect();
    assert_eq!(
        numeric,
        ["ICB", "SIC", "MULTIPLIER", "SPC", "EXPIRATION", "LAST_TRADE", "STRIKE"]
    );

    let mut profile = expected().remove(1);
    let strike = InstrumentProfileField::Strike;
    assert_eq!(strike.get_field(&profile).unwrap(), "200.5");
    assert_eq!(strike.get_numeric_field(&profile).unwrap(), 200.5);
    strike.set_numeric_field(&mut profile, 1.5).unwrap();
    assert_eq!(profile.get_field("STRIKE").unwrap(), "1.5");
    InstrumentProfileField::Icb
        .set_numeric_field(&mut profile, 3.9)
        .unwrap();
    assert_eq!(profile.icb, 3);
    InstrumentProfileField::Symbol.set_field(&mut profile, "X").unwrap();
    assert_eq!(profile.symbol, "X");
    assert!(matches!(
        InstrumentProfileField::Symbol.get_numeric_field(&profile),
        Err(Error::InvalidArgument(_))
    ));
}

#[test]
fn numeric_and_date_fields_by_name() {
    let mut profile = expected().remove(1);
    profile.set_numeric_field("STRIKE", 1.5).unwrap();
    assert_eq!(profile.get_field("STRIKE").unwrap(), "1.5");
    assert_eq!(
        profile.get_numeric_field("EXPIRATION").unwrap(),
        f64::from(DECEMBER_18_2026)
    );
    assert_eq!(profile.get_date_field("EXPIRATION").unwrap(), DECEMBER_18_2026);
    assert_eq!(profile.get_date_field("STRIKE").unwrap(), 1);
    profile.set_date_field("LAST_TRADE", DECEMBER_18_2026).unwrap();
    assert_eq!(profile.last_trade, DECEMBER_18_2026);
    assert!(matches!(
        profile.set_numeric_field("SYMBOL", 1.0),
        Err(Error::InvalidArgument(_))
    ));
    assert!(matches!(
        profile.get_date_field("SYMBOL"),
        Err(Error::InvalidArgument(_))
    ));

    profile.set_numeric_field("CUSTOM_NUMBER", 2.5).unwrap();
    assert_eq!(profile.get_field("CUSTOM_NUMBER").unwrap(), "2.5");
    assert_eq!(profile.get_numeric_field("CUSTOM_NUMBER").unwrap(), 2.5);
    profile.set_date_field("CUSTOM_DATE", DECEMBER_18_2026).unwrap();
    assert_eq!(profile.get_field("CUSTOM_DATE").unwrap(), "2026-12-18");
    assert_eq!(
        profile.get_numeric_field("CUSTOM_DATE").unwrap(),
        f64::from(DECEMBER_18_2026)
    );
    assert_eq!(profile.get_date_field("CUSTOM_DATE").unwrap(), DECEMBER_18_2026);
    assert_eq!(profile.get_numeric_field("NOT_SET").unwrap(), 0.0);
    assert_eq!(profile.get_date_field("NOT_SET").unwrap(), 0);
    profile.set_numeric_field("CUSTOM_NUMBER", 0.0).unwrap();
    assert!(!profile.custom_fields.contains_key("CUSTOM_NUMBER"));
    profile.set_field("CUSTOM_TEXT", "abc").unwrap();
    assert!(profile.get_numeric_field("CUSTOM_TEXT").is_err());

    let day = InstrumentProfileField::parse_date("2026-10-31").unwrap();
    assert_eq!(InstrumentProfileField::format_date(day).unwrap(), "2026-10-31");
    assert_eq!(InstrumentProfileField::format_date(0).unwrap(), "");
    assert_eq!(InstrumentProfileField::parse_date("").unwrap(), 0);
    assert_eq!(InstrumentProfileField::format_number(0.5).unwrap(), "0.5");
    assert_eq!(InstrumentProfileField::format_number(0.0).unwrap(), "");
    assert_eq!(InstrumentProfileField::parse_number("100").unwrap(), 100.0);
    assert!(matches!(InstrumentProfileField::parse_number("x"), Err(Error::Java(_))));
}

#[test]
fn types_and_profiles_sort_as_in_java() {
    assert_eq!(InstrumentProfileType::ALL.len(), 17);
    assert_eq!(
        InstrumentProfileType::find("MUTUAL_FUND"),
        Some(InstrumentProfileType::MutualFund)
    );
    assert_eq!(InstrumentProfileType::Removed.to_string(), InstrumentProfile::REMOVED);
    assert_eq!(
        "ETF".parse::<InstrumentProfileType>().unwrap(),
        InstrumentProfileType::Etf
    );
    assert!(matches!(
        "etf".parse::<InstrumentProfileType>(),
        Err(Error::InvalidArgument(_))
    ));
    let compare = InstrumentProfileType::compare_types;
    assert_eq!(compare("STOCK", "OPTION"), Ordering::Less);
    assert_eq!(compare("ZZZ", "REMOVED"), Ordering::Greater);
    assert_eq!(compare("AAA", "BBB"), Ordering::Less);
    assert_eq!(compare("FOREX", "FOREX"), Ordering::Equal);

    let profile = |instrument_type: &str, symbol: &str, underlying: &str, strike: f64| InstrumentProfile {
        instrument_type: instrument_type.into(),
        symbol: symbol.into(),
        underlying: underlying.into(),
        strike,
        ..InstrumentProfile::new()
    };
    let mut profiles = [
        profile("CUSTOM", "C", "", 0.0),
        profile("OPTION", "O2", "AAPL", 200.0),
        profile("OPTION", "O3", "MSFT", 100.0),
        profile("OPTION", "O1", "AAPL", 100.0),
        profile("STOCK", "B", "", 0.0),
        profile("STOCK", "A", "", 0.0),
    ];
    profiles.sort_by(InstrumentProfile::compare_to);
    let symbols: Vec<_> = profiles.iter().map(|p| p.symbol.as_str()).collect();
    assert_eq!(symbols, ["A", "B", "O1", "O2", "O3", "C"]);
    let mut described = profiles[0].clone();
    described.description = "other".into();
    assert_eq!(described.compare_to(&profiles[0]), Ordering::Equal);
}
