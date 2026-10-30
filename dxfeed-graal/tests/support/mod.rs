//! The `LOCAL_HUB` fixture: events published to a hub reach its listeners on the test thread.

#![allow(dead_code)] // Each test binary uses a part.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};

use dxfeed_graal::event::{
    Greeks, GreeksRef, NuamTrade, NuamTradeRef, PriceType, Profile, ProfileRef, Quote, QuoteRef, Series, SeriesRef,
    ShortSaleRestriction, Summary, SummaryRef, TheoPrice, TheoPriceRef, Trade, TradeETH, TradeETHRef, TradeRef,
    TradingStatus, Underlying, UnderlyingRef,
};
use dxfeed_graal::{
    DXEndpoint, DXFeed, DXFeedSubscription, DXPublisher, Event, EventRef, ManualExecutor, Role, SubscriptionItem,
};

/// A `LOCAL_HUB` endpoint with a name unique in this process, its manual executor, feed and publisher.
pub struct Hub {
    pub endpoint: DXEndpoint,
    pub executor: ManualExecutor,
    pub feed: DXFeed,
    pub publisher: DXPublisher,
}

impl Hub {
    pub fn new() -> Self {
        Self::with_executor(Some(ManualExecutor::new().unwrap()))
    }

    /// A hub whose listeners run on the endpoint's default executor threads.
    pub fn with_default_executor() -> Self {
        Self::with_executor(None)
    }

    fn with_executor(executor: Option<ManualExecutor>) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let mut builder = DXEndpoint::new_builder()
            .with_role(Role::LocalHub)
            .with_name(&format!("test-hub-{}", NEXT.fetch_add(1, Ordering::Relaxed)));
        if let Some(executor) = &executor {
            builder = builder.with_executor(executor);
        }
        let endpoint = builder.build().unwrap();
        let feed = endpoint.feed();
        let publisher = endpoint.publisher();
        // An unused executor for default-executor hubs keeps the fields uniform.
        let executor = executor.unwrap_or_else(|| ManualExecutor::new().unwrap());
        Hub {
            endpoint,
            executor,
            feed,
            publisher,
        }
    }

    /// Publishes `events` and runs the resulting listener calls on this thread.
    pub fn publish_and_process(&self, events: &[Event]) {
        self.publisher.publish_events(events).unwrap();
        self.executor.process_all_pending().unwrap();
    }
}

/// An item a listener receives, copied into an owned event.
pub trait ToEvent {
    fn to_event(&self) -> Option<Event>;
}

macro_rules! to_event {
    ($($view:ident),*) => {
        $(
            impl ToEvent for &$view {
                fn to_event(&self) -> Option<Event> {
                    Some($view::to_owned(self).into())
                }
            }
        )*
    };
}

to_event!(
    QuoteRef,
    ProfileRef,
    SummaryRef,
    GreeksRef,
    UnderlyingRef,
    TheoPriceRef,
    TradeRef,
    TradeETHRef,
    SeriesRef,
    NuamTradeRef
);

impl ToEvent for EventRef<'_> {
    fn to_event(&self) -> Option<Event> {
        self.to_owned()
    }
}

/// Adds a listener that sends copies of the received events to the returned channel; assertions belong in the test.
pub fn collect<E>(subscription: &DXFeedSubscription<E>) -> Receiver<Event>
where
    E: SubscriptionItem,
    for<'a> E::Item<'a>: ToEvent,
{
    let (sender, receiver) = mpsc::channel();
    subscription
        .add_event_listener(move |events| {
            for event in events {
                if let Some(event) = event.to_event() {
                    let _ = sender.send(event);
                }
            }
        })
        .unwrap();
    receiver
}

/// A quote with a distinct non-zero value in every field.
pub fn quote(symbol: &str) -> Quote {
    Quote {
        event_time: 1_700_000_000_500,
        time_millis_sequence: (123 << 22) | 7,
        time_nano_part: 456,
        bid_time: 1_700_000_000_000,
        bid_exchange_code: 'Q',
        bid_price: 189.5,
        bid_size: 100.0,
        ask_time: 1_700_000_001_000, // Whole seconds: the SDK truncates the time to seconds.
        ask_exchange_code: 'N',
        ask_price: 189.75,
        ask_size: 200.0,
        ..Quote::new(symbol)
    }
}

/// A trade with a distinct non-zero value in every field.
pub fn trade(symbol: &str) -> Trade {
    Trade {
        event_time: 1_700_000_000_500,
        time_sequence: (1_700_000_000_000 << 22) | 9,
        time_nano_part: 321,
        exchange_code: 'Q',
        price: 190.25,
        change: 0.5,
        size: 300.0,
        day_id: 19_675,
        day_volume: 1_000_000.0,
        day_turnover: 190_000_000.0,
        flags: 4,
        trade_id: 42,
        ..Trade::new(symbol)
    }
}

/// A trade in extended trading hours with the values of [`trade`].
pub fn trade_eth(symbol: &str) -> TradeETH {
    let t = trade(symbol);
    TradeETH {
        event_time: t.event_time,
        time_sequence: t.time_sequence,
        time_nano_part: t.time_nano_part,
        exchange_code: t.exchange_code,
        price: t.price,
        change: t.change,
        size: t.size,
        day_id: t.day_id,
        day_volume: t.day_volume,
        day_turnover: t.day_turnover,
        flags: t.flags | 1,
        trade_id: t.trade_id,
        ..TradeETH::new(symbol)
    }
}

/// A Nuam trade with the values of [`trade`] and distinct non-zero statistics.
pub fn nuam_trade(symbol: &str) -> NuamTrade {
    let t = trade(symbol);
    NuamTrade {
        event_time: t.event_time,
        time_sequence: t.time_sequence,
        time_nano_part: t.time_nano_part,
        exchange_code: t.exchange_code,
        price: t.price,
        change: t.change,
        size: t.size,
        day_id: t.day_id,
        day_volume: t.day_volume,
        day_turnover: t.day_turnover,
        flags: t.flags,
        trade_id: t.trade_id,
        trade_stat_time: 1_700_000_002_000,
        last_significant_price: 190.5,
        last_price_for_all: 190.75,
        number_of_trades: 12_345,
        vwap: 190.125,
        ..NuamTrade::new(symbol)
    }
}

/// A profile with a distinct non-zero value in every field.
pub fn profile(symbol: &str) -> Profile {
    let mut profile = Profile {
        event_time: 1_700_000_000_500,
        description: Some("Apple Inc.".to_owned()),
        status_reason: Some("news".to_owned()),
        halt_start_time: 1_700_000_000_000,
        halt_end_time: 1_700_000_100_000,
        high_limit_price: 200.0,
        low_limit_price: 150.0,
        high_52_week_price: 199.5,
        low_52_week_price: 120.25,
        beta: 1.25,
        earnings_per_share: 6.5,
        dividend_frequency: 4.0,
        ex_dividend_amount: 0.24,
        ex_dividend_day_id: 19_600,
        shares: 15_500_000_000.0,
        free_float: 15_400_000_000.0,
        ..Profile::new(symbol)
    };
    profile.set_short_sale_restriction(ShortSaleRestriction::Active);
    profile.set_trading_status(TradingStatus::Halted);
    profile
}

/// A summary with a distinct non-zero value in every field.
pub fn summary(symbol: &str) -> Summary {
    let mut summary = Summary {
        event_time: 1_700_000_000_500,
        day_id: 19_675,
        day_open_price: 189.0,
        day_high_price: 191.5,
        day_low_price: 188.25,
        day_close_price: 190.0,
        prev_day_id: 19_674,
        prev_day_close_price: 188.5,
        prev_day_volume: 55_000_000.0,
        open_interest: 1234,
        ..Summary::new(symbol)
    };
    summary.set_day_close_price_type(PriceType::Final);
    summary.set_prev_day_close_price_type(PriceType::Preliminary);
    summary
}

/// The time and sequence of the samples of time series events: whole seconds and the sequence 5.
const INDEX: i64 = (1_700_000_000 << 32) | 5;

/// Greeks with a distinct non-zero value in every field but the event flags.
pub fn greeks(symbol: &str) -> Greeks {
    Greeks {
        event_time: 1_700_000_000_500,
        index: INDEX,
        price: 12.5,
        volatility: 0.3,
        delta: 0.55,
        gamma: 0.02,
        theta: -0.05,
        rho: 0.1,
        vega: 0.2,
        ..Greeks::new(symbol)
    }
}

/// An underlying with a distinct non-zero value in every field but the event flags.
pub fn underlying(symbol: &str) -> Underlying {
    Underlying {
        event_time: 1_700_000_000_500,
        index: INDEX,
        volatility: 0.25,
        front_volatility: 0.24,
        back_volatility: 0.26,
        call_volume: 1000.0,
        put_volume: 800.0,
        put_call_ratio: 0.8,
        ..Underlying::new(symbol)
    }
}

/// A theoretical price with a distinct non-zero value in every field but the event flags.
pub fn theo_price(symbol: &str) -> TheoPrice {
    TheoPrice {
        event_time: 1_700_000_000_500,
        index: INDEX,
        price: 12.4,
        underlying_price: 190.0,
        delta: 0.56,
        gamma: 0.03,
        dividend: 0.01,
        interest: 0.05,
        ..TheoPrice::new(symbol)
    }
}

/// A series with a distinct non-zero value in every field but the event flags.
pub fn series(symbol: &str) -> Series {
    Series {
        event_time: 1_700_000_000_500,
        index: 17,
        time_sequence: INDEX,
        expiration: 19_700,
        volatility: 0.27,
        call_volume: 1100.0,
        put_volume: 900.0,
        put_call_ratio: 0.81,
        forward_price: 190.5,
        dividend: 0.02,
        interest: 0.04,
        ..Series::new(symbol)
    }
}

/// One event of each mapped type, with the values of the samples.
pub fn all_events(symbol: &str) -> Vec<Event> {
    vec![
        quote(symbol).into(),
        profile(symbol).into(),
        summary(symbol).into(),
        greeks(symbol).into(),
        underlying(symbol).into(),
        theo_price(symbol).into(),
        trade(symbol).into(),
        trade_eth(symbol).into(),
        series(symbol).into(),
        nuam_trade(symbol).into(),
    ]
}

/// The event as `LOCAL_HUB` delivers it: the SDK does not carry through the hub the event time of any event, the
/// millisecond-sequence of a quote and the nanosecond part of the trades; they come back as zero.
pub fn delivered(event: Event) -> Event {
    match event {
        Event::Quote(e) => Quote {
            event_time: 0,
            time_millis_sequence: 0,
            time_nano_part: 0,
            ..e
        }
        .into(),
        Event::Profile(e) => Profile { event_time: 0, ..e }.into(),
        Event::Summary(e) => Summary { event_time: 0, ..e }.into(),
        Event::Greeks(e) => Greeks { event_time: 0, ..e }.into(),
        Event::Underlying(e) => Underlying { event_time: 0, ..e }.into(),
        Event::TheoPrice(e) => TheoPrice { event_time: 0, ..e }.into(),
        Event::Trade(e) => Trade {
            event_time: 0,
            time_nano_part: 0,
            ..e
        }
        .into(),
        Event::TradeETH(e) => TradeETH {
            event_time: 0,
            time_nano_part: 0,
            ..e
        }
        .into(),
        Event::Series(e) => Series { event_time: 0, ..e }.into(),
        Event::NuamTrade(e) => NuamTrade {
            event_time: 0,
            time_nano_part: 0,
            ..e
        }
        .into(),
        other => panic!("no delivered form of {other}"),
    }
}

/// [`quote`] with another bid price: the hub does not deliver a quote identical to the last one.
pub fn quote_with_bid(symbol: &str, bid_price: f64) -> Quote {
    Quote {
        bid_price,
        ..quote(symbol)
    }
}

fn digits(s: &str, n: usize) -> bool {
    s.len() == n && s.bytes().all(|b| b.is_ascii_digit())
}

/// Whether `text` is a time of `TimeFormat.DEFAULT.withMillis()` (`yyyyMMdd-HHmmss.SSS`).
pub fn is_time(text: &str) -> bool {
    text.split_once('.')
        .is_some_and(|(seconds, millis)| is_time_without_millis(seconds) && digits(millis, 3))
}

/// Whether `text` is a time of `TimeFormat.DEFAULT` (`yyyyMMdd-HHmmss`).
pub fn is_time_without_millis(text: &str) -> bool {
    text.split_once('-').is_some_and(|(d, t)| digits(d, 8) && digits(t, 6))
}
