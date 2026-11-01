//! The Java counterparts that the coverage re-audit added: completing a promise, the filters of days and sessions, the
//! equality of instrument profiles, the constructors of the text events, the scopes of the order book filters, the
//! Java names of the candle attributes, and attaching a closed model; no network needed.

mod support;

use std::hash::{BuildHasher, RandomState};
use std::sync::{Mutex, MutexGuard, PoisonError, mpsc};
use std::time::Duration;

use dxfeed_graal::event::{
    BaseEvent, Candle, CandleAlignment, CandlePrice, CandleSession, CandleType, Configuration, Message, Order,
    OrderSource, Quote, Scope, TextConfiguration, TextMessage,
};
use dxfeed_graal::ipf::InstrumentProfile;
use dxfeed_graal::model::{
    IndexedEventModel, IndexedTxModel, OrderBookModel, OrderBookModelFilter, TimeSeriesEventModel,
};
use dxfeed_graal::schedule::{DayFilter, Schedule, SessionFilter};
use dxfeed_graal::{DXEndpoint, Role, live_handles};
use support::{Hub, order, quote_with_bid};

/// Runs the tests of this file one by one, so that each can count the live handles.
fn serial() -> MutexGuard<'static, ()> {
    static SERIAL: Mutex<()> = Mutex::new(());
    SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
}

const LONG: Duration = Duration::from_secs(10);

/// The schedule of `tests/schedule.rs`: New York, trading Monday to Friday, no holidays.
const DEFINITION: &str = "Test(name=Test;tz=America/New_York;td=12345;de=+0000;0=p04000930r09301600a16002000)";

const DAY_FILTERS: [DayFilter; 14] = [
    DayFilter::Any,
    DayFilter::Trading,
    DayFilter::NonTrading,
    DayFilter::Holiday,
    DayFilter::ShortDay,
    DayFilter::Monday,
    DayFilter::Tuesday,
    DayFilter::Wednesday,
    DayFilter::Thursday,
    DayFilter::Friday,
    DayFilter::Saturday,
    DayFilter::Sunday,
    DayFilter::WeekDay,
    DayFilter::WeekEnd,
];

const SESSION_FILTERS: [SessionFilter; 7] = [
    SessionFilter::Any,
    SessionFilter::Trading,
    SessionFilter::NonTrading,
    SessionFilter::NoTrading,
    SessionFilter::PreMarket,
    SessionFilter::Regular,
    SessionFilter::AfterMarket,
];

#[test]
fn complete_gives_a_promise_its_event_once() {
    let _serial = serial();
    let hub = Hub::new();
    let promise = hub.feed.get_last_event_promise::<Quote>("AAPL").unwrap();
    let (sender, done) = mpsc::channel();
    promise
        .when_done(move |promise| {
            let _ = sender.send(promise.get_result().unwrap().map(|quote| quote.bid_price));
        })
        .unwrap();
    assert!(!promise.is_done().unwrap());

    promise.complete(&quote_with_bid("AAPL", 101.5)).unwrap();
    assert!(promise.is_done().unwrap());
    // The handlers of a feed request run on the executor of the endpoint.
    hub.executor.process_all_pending().unwrap();
    assert_eq!(done.recv_timeout(LONG).unwrap(), Some(101.5));
    // A done promise keeps its result.
    promise.complete(&quote_with_bid("AAPL", 7.0)).unwrap();
    assert_eq!(promise.get_result().unwrap().unwrap().bid_price, 101.5);
}

#[test]
fn complete_gives_a_list_promise_its_events() {
    let _serial = serial();
    let hub = Hub::new();
    let promise = hub
        .feed
        .get_indexed_events_promise::<Order>("BOOK", OrderSource::NTV)
        .unwrap();
    assert!(!promise.is_done().unwrap());
    let orders = [
        Order {
            index: 2,
            ..order("BOOK")
        },
        Order {
            index: 1,
            ..order("BOOK")
        },
    ];

    promise.complete(&orders).unwrap();
    let result = promise.wait(LONG).unwrap();
    assert_eq!(result.iter().map(|order| order.index).collect::<Vec<_>>(), [2, 1]);
    assert!(result.iter().all(|order| order.symbol() == "BOOK"));
    promise.complete(&[]).unwrap();
    assert_eq!(promise.get_result().unwrap().unwrap().len(), 2);
}

/// Each filter accepts the days that the SDK's own filter finds.
#[test]
fn day_filters_accept_the_days_of_the_sdk_filters() {
    let _serial = serial();
    let schedule = Schedule::get_instance_by_definition(DEFINITION).unwrap();
    // A Sunday, before a whole week.
    let start = schedule.get_day_by_year_month_day(20261025).unwrap();
    let mut days = vec![start.get_next_day(DayFilter::Any).unwrap()];
    while days.len() < 14 {
        days.push(days.last().unwrap().get_next_day(DayFilter::Any).unwrap());
    }
    for filter in DAY_FILTERS {
        let ours = days.iter().find(|day| filter.accept(day).unwrap());
        let sdk = start.find_next_day(filter).unwrap();
        match ours {
            Some(day) => assert_eq!(Some(day), sdk.as_ref(), "{filter:?}"),
            // No holidays and no short days in two weeks: the SDK finds none there either.
            None => assert!(sdk.is_none_or(|day| !days.contains(&day)), "{filter:?}"),
        }
    }
    let wednesday = schedule.get_day_by_year_month_day(20261028).unwrap();
    let saturday = schedule.get_day_by_year_month_day(20261031).unwrap();
    assert!(DayFilter::Wednesday.accept(&wednesday).unwrap());
    assert!(DayFilter::WeekDay.accept(&wednesday).unwrap());
    assert!(DayFilter::WeekEnd.accept(&saturday).unwrap());
    assert!(DayFilter::NonTrading.accept(&saturday).unwrap());
    assert!(!DayFilter::Trading.accept(&saturday).unwrap());
}

/// A holiday and a short day, which the schedule above has none of.
#[test]
fn day_filters_accept_holidays_and_short_days() {
    let _serial = serial();
    let schedule = Schedule::get_instance_by_definition(
        "Test(name=Test;tz=America/New_York;hd=20261029;sd=20261030;td=12345;de=+0000;0=p04000930r09301600a16002000)",
    )
    .unwrap();
    let holiday = schedule.get_day_by_year_month_day(20261029).unwrap();
    let short_day = schedule.get_day_by_year_month_day(20261030).unwrap();
    let wednesday = schedule.get_day_by_year_month_day(20261028).unwrap();
    assert!(DayFilter::Holiday.accept(&holiday).unwrap());
    assert!(DayFilter::NonTrading.accept(&holiday).unwrap());
    assert!(DayFilter::ShortDay.accept(&short_day).unwrap());
    assert!(!DayFilter::Holiday.accept(&short_day).unwrap());
    assert!(!DayFilter::ShortDay.accept(&wednesday).unwrap());
    assert_eq!(wednesday.find_next_day(DayFilter::Holiday).unwrap(), Some(holiday));
    assert_eq!(wednesday.find_next_day(DayFilter::ShortDay).unwrap(), Some(short_day));
}

/// Each filter accepts the first session that the SDK's own filter finds, and none before it.
#[test]
fn session_filters_accept_the_sessions_of_the_sdk_filters() {
    let _serial = serial();
    let schedule = Schedule::get_instance_by_definition(DEFINITION).unwrap();
    for year_month_day in [20261028, 20261031] {
        let day = schedule.get_day_by_year_month_day(year_month_day).unwrap();
        let sessions = day.get_sessions().unwrap();
        for filter in SESSION_FILTERS {
            let ours = sessions.iter().find(|session| filter.accept(session).unwrap());
            assert_eq!(
                ours,
                day.find_first_session(filter).unwrap().as_ref(),
                "{filter:?} on {year_month_day}"
            );
        }
    }
}

/// Profiles are equal and hash as in Java: NaN equals NaN, the zeros differ, an empty custom value is missing.
#[test]
fn instrument_profiles_are_equal_as_in_java() {
    let hasher = RandomState::new();
    let profile = InstrumentProfile {
        instrument_type: "STOCK".into(),
        symbol: "AAPL".into(),
        strike: f64::NAN,
        ..InstrumentProfile::default()
    };
    let same = |change: fn(&mut InstrumentProfile)| {
        let mut other = profile.clone();
        change(&mut other);
        let equal = other == profile;
        if equal {
            assert_eq!(hasher.hash_one(&other), hasher.hash_one(&profile));
        }
        equal
    };

    assert!(same(|_| {}));
    assert!(same(|other| other.strike = -f64::NAN));
    assert!(same(|other| {
        other.custom_fields.insert("EMPTY".into(), String::new());
    }));
    assert!(!same(|other| other.multiplier = -0.0));
    assert!(!same(|other| {
        other.custom_fields.insert("NAME".into(), "value".into());
    }));
    assert!(!same(|other| other.symbol = "IBM".into()));
}

#[test]
fn text_events_have_the_constructors_of_java() {
    let message = Message::with_attachment("AAPL", "{\"a\":1}");
    assert_eq!(message.symbol(), "AAPL");
    assert_eq!(message.attachment.as_deref(), Some("{\"a\":1}"));

    let configuration = Configuration::with_attachment("AAPL", "config", 3);
    assert_eq!(configuration.symbol(), "AAPL");
    assert_eq!(configuration.attachment.as_deref(), Some("config"));
    assert_eq!(configuration.version, 3);

    let text_configuration = TextConfiguration::with_text("AAPL", "text", 4);
    assert_eq!(text_configuration.symbol(), "AAPL");
    assert_eq!(text_configuration.text.as_deref(), Some("text"));
    assert_eq!(text_configuration.version, 4);

    let text_message = TextMessage::with_text("AAPL", 1_700_000_000_123, "hello");
    assert_eq!(text_message.symbol(), "AAPL");
    assert_eq!(text_message.time(), 1_700_000_000_123);
    assert_eq!(text_message.text.as_deref(), Some("hello"));
    assert_eq!((text_message.sequence(), text_message.event_time), (0, 0));
}

/// The scopes of each filter, as in Java's constructors of `OrderBookModelFilter`.
#[test]
fn order_book_filters_allow_the_scopes_of_java() {
    use Scope::{Aggregate, Composite, Order, Regional};
    let expected: [(OrderBookModelFilter, &[Scope]); 7] = [
        (OrderBookModelFilter::Composite, &[Composite]),
        (OrderBookModelFilter::Regional, &[Regional]),
        (OrderBookModelFilter::Aggregate, &[Aggregate]),
        (OrderBookModelFilter::Order, &[Order]),
        (OrderBookModelFilter::CompositeRegional, &[Composite, Regional]),
        (
            OrderBookModelFilter::CompositeRegionalAggregate,
            &[Composite, Regional, Aggregate],
        ),
        (OrderBookModelFilter::All, &[Composite, Regional, Aggregate, Order]),
    ];
    assert_eq!(expected.map(|(filter, _)| filter), OrderBookModelFilter::ALL);
    for (filter, scopes) in expected {
        for &scope in Scope::ALL {
            assert_eq!(
                filter.allow_scope(scope),
                scopes.contains(&scope),
                "{filter:?} and {scope:?}"
            );
        }
    }
}

/// The names of the values in the order of Java's `values()`.
#[test]
fn candle_attributes_have_the_java_names() {
    fn names<T: Copy>(all: &[T], name: fn(T) -> &'static str) -> Vec<&'static str> {
        all.iter().map(|&value| name(value)).collect()
    }
    assert_eq!(
        names(CandleType::ALL, CandleType::get_name),
        [
            "TICK",
            "SECOND",
            "MINUTE",
            "HOUR",
            "DAY",
            "WEEK",
            "MONTH",
            "OPTEXP",
            "YEAR",
            "VOLUME",
            "PRICE",
            "PRICE_MOMENTUM",
            "PRICE_RENKO",
        ]
    );
    assert_eq!(
        names(CandlePrice::ALL, CandlePrice::get_name),
        ["LAST", "BID", "ASK", "MARK", "SETTLEMENT"]
    );
    assert_eq!(names(CandleSession::ALL, CandleSession::get_name), ["ANY", "REGULAR"]);
    assert_eq!(
        names(CandleAlignment::ALL, CandleAlignment::get_name),
        ["MIDNIGHT", "SESSION"]
    );
}

/// Attaching a closed model to a feed does not keep the feed, nor with it the endpoint.
#[test]
fn attaching_a_closed_model_keeps_no_feed() {
    let _serial = serial();
    let order_book = OrderBookModel::new().unwrap();
    let indexed = IndexedEventModel::<Order>::new().unwrap();
    let time_series = TimeSeriesEventModel::<Candle>::new().unwrap();
    let tx = IndexedTxModel::<Order>::new_builder()
        .with_symbol("TX")
        .with_sources([OrderSource::NTV])
        .build()
        .unwrap();
    order_book.close().unwrap();
    indexed.close().unwrap();
    time_series.close().unwrap();
    tx.close().unwrap();
    let closed = live_handles();

    let endpoint = DXEndpoint::new_builder()
        .with_role(Role::LocalHub)
        .with_name("closed-models")
        .build()
        .unwrap();
    let feed = endpoint.get_feed();
    order_book.attach(&feed).unwrap();
    indexed.attach(&feed).unwrap();
    time_series.attach(&feed).unwrap();
    tx.attach(&feed).unwrap();
    endpoint.close().unwrap();
    drop((feed, endpoint));
    assert_eq!(live_handles(), closed);
    drop((order_book, indexed, time_series, tx));
}
