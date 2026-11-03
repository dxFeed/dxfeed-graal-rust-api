//! Feed filters by categories of QD filter expressions, with the cargo feature `experimental`; no network needed.

#![cfg(feature = "experimental")]

use std::sync::mpsc;
use std::time::Duration;

use dxfeed_graal::event::{EventType, Quote, Trade};
use dxfeed_graal::feed_filter::DXFeedFilter;
use dxfeed_graal::{DXEndpoint, Error, Event, Role};

fn endpoint() -> DXEndpoint {
    DXEndpoint::create_with_role(Role::LocalHub).unwrap()
}

fn is_java(result: Result<impl std::fmt::Debug, Error>, class_name: &str) -> bool {
    matches!(result, Err(Error::Java(e)) if e.get_class_name() == class_name)
}

fn is_illegal_argument(result: Result<impl std::fmt::Debug, Error>) -> bool {
    is_java(result, "java.lang.IllegalArgumentException")
}

#[test]
fn the_first_category_that_passes_names_the_subscription() {
    let endpoint = endpoint();
    let builder = DXFeedFilter::new_builder(&endpoint)
        .with_category("big", "IBM,AAPL")
        .with_category("rest", "*");
    assert_eq!(
        builder.get_categories(),
        [
            ("big".to_owned(), "IBM,AAPL".to_owned()),
            ("rest".to_owned(), "*".to_owned())
        ]
    );
    assert_eq!(builder.get_endpoint().get_role(), Role::LocalHub);
    let filter = builder.build().unwrap();

    assert!(filter.accept(EventType::Quote, "IBM").unwrap());
    assert_eq!(
        filter.get_category(EventType::Quote, "IBM").unwrap().as_deref(),
        Some("big")
    );
    assert_eq!(
        filter.get_category(EventType::Trade, "AAPL").unwrap().as_deref(),
        Some("big")
    );
    assert!(filter.accept(EventType::Quote, "MSFT").unwrap());
    assert_eq!(
        filter.get_category(EventType::Quote, "MSFT").unwrap().as_deref(),
        Some("rest")
    );
}

#[test]
fn events_pass_as_their_symbols_do() {
    let filter = DXFeedFilter::new_builder(&endpoint())
        .with_categories([("big", "IBM,AAPL")])
        .build()
        .unwrap();
    for (event, category) in [
        (Event::from(Quote::new("IBM")), Some("big")),
        (Event::from(Trade::new("AAPL")), Some("big")),
        (Event::from(Quote::new("MSFT")), None),
    ] {
        let event_type = event.event_type();
        let symbol = event.symbol().to_owned();
        assert_eq!(
            filter.get_event_category(&event).unwrap().as_deref(),
            category,
            "{event:?}"
        );
        assert_eq!(
            filter.get_category(event_type, symbol.as_str()).unwrap().as_deref(),
            category
        );
        assert_eq!(filter.accept_event(&event).unwrap(), category.is_some());
        assert_eq!(filter.accept(event_type, symbol.as_str()).unwrap(), category.is_some());
    }
}

#[test]
fn a_static_filter_has_a_tracker_of_the_same_filter() {
    let filter = DXFeedFilter::new_builder(&endpoint())
        .with_category("big", "IBM,AAPL")
        .with_category("rest", "*")
        .build()
        .unwrap();
    assert!(!filter.is_dynamic().unwrap());
    let tracker = filter.get_tracker().unwrap();
    let (sender, updates) = mpsc::channel();
    let id = tracker.add_listener(move || sender.send(()).unwrap()).unwrap();
    let current = tracker.get_current_filter().unwrap();
    assert_eq!(current.to_string(), filter.to_string());
    assert_eq!(
        current.get_category(EventType::Quote, "IBM").unwrap().as_deref(),
        Some("big")
    );
    assert!(!current.is_dynamic().unwrap());
    tracker.remove_listener(id).unwrap();
    assert!(tracker.remove_listener(id).is_err());
    assert!(updates.try_recv().is_err(), "a static filter is never updated");
    assert!(format!("{tracker:?} {filter:?}").starts_with("DXFeedFilterTracker"));
}

#[test]
fn a_dynamic_filter_tells_its_listeners_of_updates() {
    let dir = std::env::temp_dir().join(format!("dxfeed-graal-feed-filter-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("symbols.ipf");
    let write = |symbol: &str| {
        let text = format!("#STOCK::=TYPE,SYMBOL\nSTOCK,{symbol}\n##COMPLETE\n");
        std::fs::write(&file, text).unwrap();
    };
    write("IBM");
    let expression = format!("ipf[{},update=1s]", file.to_str().unwrap().replace('\\', "/"));
    let filter = DXFeedFilter::new_builder(&endpoint())
        .with_category("listed", &expression)
        .build()
        .unwrap();
    assert!(filter.is_dynamic().unwrap());
    assert_eq!(
        filter.get_category(EventType::Quote, "IBM").unwrap().as_deref(),
        Some("listed")
    );
    assert_eq!(filter.get_category(EventType::Quote, "MSFT").unwrap(), None);

    let tracker = filter.get_tracker().unwrap();
    let (sender, updates) = mpsc::channel();
    tracker.add_listener(move || sender.send(()).unwrap()).unwrap();
    // A later modification time than the first version's.
    std::thread::sleep(Duration::from_millis(1_500));
    write("MSFT");
    updates
        .recv_timeout(Duration::from_secs(10))
        .expect("no update of the filter");
    let current = tracker.get_current_filter().unwrap();
    assert_eq!(current.get_category(EventType::Quote, "IBM").unwrap(), None);
    assert_eq!(
        current.get_category(EventType::Quote, "MSFT").unwrap().as_deref(),
        Some("listed")
    );
    assert_eq!(
        filter.get_category(EventType::Quote, "IBM").unwrap().as_deref(),
        Some("listed"),
        "a filter does not change"
    );
    drop(tracker);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn invalid_categories_are_refused() {
    let endpoint = endpoint();
    assert!(is_illegal_argument(DXFeedFilter::new_builder(&endpoint).build()));
    assert!(is_illegal_argument(
        DXFeedFilter::new_builder(&endpoint).with_category("empty", "").build()
    ));
    assert!(is_java(
        DXFeedFilter::new_builder(&endpoint)
            .with_category("bad", "IBM(")
            .build(),
        "com.devexperts.qd.kit.FilterSyntaxException"
    ));
    assert!(is_illegal_argument(
        DXFeedFilter::new_builder(&endpoint)
            .with_category("same", "IBM")
            .with_category("same", "AAPL")
            .build()
    ));
    assert!(matches!(
        DXFeedFilter::new_builder(&endpoint)
            .with_category("nul\0", "IBM")
            .build(),
        Err(Error::InvalidArgument(_))
    ));
}
