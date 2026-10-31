//! Instrument profiles: reading a file, fields by name, the collector and a connection to a file; no network needed.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use dxfeed_graal::Error;
use dxfeed_graal::ipf::InstrumentProfile;
use dxfeed_graal::ipf::InstrumentProfileReader;
use dxfeed_graal::ipf::live::{ConnectionState, InstrumentProfileCollector, InstrumentProfileConnection};

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
    assert!(reader.last_modified().unwrap() > 0);
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
    assert_eq!(profile.field("SYMBOL").unwrap(), ".AAPL261218C200");
    assert_eq!(profile.field("STRIKE").unwrap(), "200.5");
    assert_eq!(profile.field("SPC").unwrap(), "100");
    assert_eq!(profile.field("ICB").unwrap(), "");
    assert_eq!(profile.field("EXPIRATION").unwrap(), "2026-12-18");
    assert_eq!(profile.field("NO_SUCH_FIELD").unwrap(), "");

    profile.set_field("LAST_TRADE", "2026-12-17").unwrap();
    assert_eq!(profile.last_trade, DECEMBER_18_2026 - 1);
    profile.set_field("SIC", "1234").unwrap();
    assert_eq!(profile.sic, 1234);
    profile.set_field("NOTE", "x").unwrap();
    assert_eq!(profile.field("NOTE").unwrap(), "x");
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
    assert!(collector.last_update_time().unwrap() > 0);

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
    assert_eq!(connection.address().unwrap(), fixture());
    assert_eq!(connection.state().unwrap(), ConnectionState::NotConnected);
    connection.set_update_period(Duration::from_secs(30)).unwrap();
    assert_eq!(connection.update_period().unwrap(), Duration::from_secs(30));
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
    assert_eq!(connection.state().unwrap(), ConnectionState::Completed);
    // Java records the modification time just after it reports COMPLETED.
    let deadline = Instant::now() + Duration::from_secs(10);
    while connection.last_modified().unwrap() == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(connection.last_modified().unwrap() > 0);
    assert!(connection.start().is_err());

    connection.close().unwrap();
    assert_eq!(next(), ConnectionState::Closed);
    assert_eq!(ConnectionState::NotConnected.to_string(), "NOT_CONNECTED");
}
