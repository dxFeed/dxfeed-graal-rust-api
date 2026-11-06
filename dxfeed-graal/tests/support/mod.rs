//! The `LOCAL_HUB` fixture: events published to a hub reach its listeners on the test thread.

#![allow(dead_code)] // Each test binary uses a part.

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use dxfeed_graal::event::{
    AnalyticOrder, AnalyticOrderRef, AuctionType, Candle, CandleRef, Configuration, ConfigurationRef, DailyCandle,
    DailyCandleRef, Greeks, GreeksRef, IcebergType, ImbalanceSide, MarketMaker, MarketMakerRef, Message, MessageRef,
    NuamOrder, NuamOrderRef, NuamOrderType, NuamTimeAndSale, NuamTimeAndSaleRef, NuamTimeInForceType, NuamTrade,
    NuamTradeRef, OptionSale, OptionSaleRef, Order, OrderAction, OrderBase, OrderBaseRef, OrderImbalance,
    OrderImbalanceRef, OrderRef, OtcMarketsOrder, OtcMarketsOrderRef, OtcMarketsPriceType, PriceType, Profile,
    ProfileRef, Quote, QuoteRef, Scope, Series, SeriesRef, ShortSaleRestriction, Side, SpreadOrder, SpreadOrderRef,
    Summary, SummaryRef, TextConfiguration, TextConfigurationRef, TextMessage, TextMessageRef, TheoPrice, TheoPriceRef,
    TimeAndSale, TimeAndSaleRef, TimeAndSaleType, Trade, TradeETH, TradeETHRef, TradeRef, TradingStatus, Underlying,
    UnderlyingRef,
};
use dxfeed_graal::{
    DXEndpoint, DXFeed, DXFeedSubscription, DXPublisher, Event, EventRef, ManualExecutor, Role, SubscriptionItem,
};

/// Set for the child processes; the scenarios do nothing without it.
pub const CHILD: &str = "DXFEED_GRAAL_TEST_CHILD";

/// Runs the ignored test `scenario` of this test executable in a child process, for the settings global to a process,
/// and returns its stdout and stderr; panics if it fails, runs no test or does not exit within 60 seconds. The
/// scenario returns at once unless [`is_child`].
pub fn run_child(scenario: &str) -> (String, String) {
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([scenario, "--exact", "--ignored", "--nocapture"])
        .env(CHILD, "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let read = |mut pipe: Box<dyn Read + Send>| {
        thread::spawn(move || {
            // Bytes that are not UTF-8 are replaced rather than losing the whole output.
            let mut bytes = Vec::new();
            let _ = pipe.read_to_end(&mut bytes);
            String::from_utf8_lossy(&bytes).into_owned()
        })
    };
    let stdout = read(Box::new(child.stdout.take().unwrap()));
    let stderr = read(Box::new(child.stderr.take().unwrap()));
    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("{scenario} did not exit within 60 seconds");
        }
        thread::sleep(Duration::from_millis(50));
    };
    let (stdout, stderr) = (stdout.join().unwrap(), stderr.join().unwrap());
    assert!(status.success(), "{scenario} failed with {status}\n{stdout}\n{stderr}");
    // A scenario name that matches no test makes libtest run nothing and exit with 0. The header is printed before the
    // test starts, so it is also there when the scenario leaves through `process::exit`.
    assert!(
        stdout.contains("running 1 test\n"),
        "{scenario} did not run exactly one test\n{stdout}\n{stderr}"
    );
    (stdout, stderr)
}

/// Whether this process is a child that runs a scenario.
pub fn is_child() -> bool {
    std::env::var_os(CHILD).is_some()
}

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
        let feed = endpoint.get_feed();
        let publisher = endpoint.get_publisher();
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
    NuamTradeRef,
    TimeAndSaleRef,
    NuamTimeAndSaleRef,
    OptionSaleRef,
    CandleRef,
    DailyCandleRef,
    OrderBaseRef,
    OrderRef,
    AnalyticOrderRef,
    OtcMarketsOrderRef,
    SpreadOrderRef,
    NuamOrderRef,
    MarketMakerRef,
    OrderImbalanceRef,
    ConfigurationRef,
    MessageRef,
    TextMessageRef,
    TextConfigurationRef
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

/// A time and sale with a distinct non-zero value in every field but the event flags.
pub fn time_and_sale(symbol: &str) -> TimeAndSale {
    let mut sale = TimeAndSale {
        event_time: 1_700_000_000_500,
        index: INDEX,
        time_nano_part: 7,
        exchange_code: 'Q',
        price: 190.5,
        size: 300.0,
        bid_price: 190.25,
        ask_price: 190.75,
        exchange_sale_conditions: Some("F01".to_owned()),
        buyer: Some("buyer_1".to_owned()),
        seller: Some("seller_1".to_owned()),
        trade_id: 4242,
        ..TimeAndSale::new(symbol)
    };
    sale.set_trade_through_exempt('X').unwrap();
    sale.set_aggressor_side(Side::Buy);
    sale.set_valid_tick(true);
    sale.set_time_and_sale_type(TimeAndSaleType::Correction);
    sale
}

/// A Nuam time and sale with a distinct non-zero value in every field but the event flags.
pub fn nuam_time_and_sale(symbol: &str) -> NuamTimeAndSale {
    let sale = time_and_sale(symbol);
    NuamTimeAndSale {
        event_time: sale.event_time,
        index: sale.index,
        time_nano_part: sale.time_nano_part,
        exchange_code: sale.exchange_code,
        price: sale.price,
        size: sale.size,
        bid_price: sale.bid_price,
        ask_price: sale.ask_price,
        exchange_sale_conditions: sale.exchange_sale_conditions,
        flags: sale.flags,
        trade_id: sale.trade_id,
        buyer: sale.buyer,
        seller: sale.seller,
        match_id: 99,
        ..NuamTimeAndSale::new(symbol)
    }
}

/// An option sale with a distinct non-zero value in every field but the event flags.
pub fn option_sale(symbol: &str) -> OptionSale {
    let mut sale = OptionSale {
        event_time: 1_700_000_000_500,
        index: 17,
        time_sequence: INDEX,
        time_nano_part: 7,
        exchange_code: 'C',
        price: 2.5,
        size: 10.0,
        bid_price: 2.45,
        ask_price: 2.55,
        exchange_sale_conditions: Some("F02".to_owned()),
        underlying_price: 190.0,
        volatility: 0.3,
        delta: 0.45,
        option_symbol: Some(".AAPL250117C150".to_owned()),
        ..OptionSale::new(symbol)
    };
    sale.set_aggressor_side(Side::Sell);
    sale.set_spread_leg(true);
    sale
}

/// A candle with a distinct non-zero value in every field but the event flags.
pub fn candle(symbol: &str) -> Candle {
    Candle {
        event_time: 1_700_000_000_500,
        index: INDEX,
        count: 120,
        open: 189.0,
        high: 191.0,
        low: 188.5,
        close: 190.5,
        volume: 50_000.0,
        vwap: 190.1,
        bid_volume: 20_000.0,
        ask_volume: 30_000.0,
        imp_volatility: 0.28,
        open_interest: 7000.0,
        ..Candle::new(symbol)
    }
}

/// A daily candle with the values of [`candle`].
pub fn daily_candle(symbol: &str) -> DailyCandle {
    let candle = candle(symbol);
    DailyCandle {
        event_time: candle.event_time,
        index: candle.index,
        count: candle.count,
        open: candle.open,
        high: candle.high,
        low: candle.low,
        close: candle.close,
        volume: candle.volume,
        vwap: candle.vwap,
        bid_volume: candle.bid_volume,
        ask_volume: candle.ask_volume,
        imp_volatility: candle.imp_volatility,
        open_interest: candle.open_interest,
        ..DailyCandle::new(symbol)
    }
}

/// An order of type `$type` from the default source with a distinct non-zero value in every field of `OrderBase` but
/// the event flags, and the fields given.
macro_rules! sample_order {
    ($type:ident($symbol:expr) { $($field:ident: $value:expr,)* }) => {{
        let mut order = $type {
            event_time: 1_700_000_000_500,
            index: 17,
            time_sequence: INDEX,
            time_nano_part: 7,
            action_time: 1_700_000_000_400,
            order_id: 1001,
            aux_order_id: 1002,
            price: 189.5,
            size: 300.0,
            executed_size: 100.0,
            count: 4,
            trade_id: 42,
            trade_price: 189.25,
            trade_size: 50.0,
            $($field: $value,)*
            ..$type::new($symbol)
        };
        order.set_action(OrderAction::Partial);
        order.set_exchange_code('Q').unwrap();
        order.set_order_side(Side::Buy);
        order.set_scope(Scope::Order);
        order
    }};
}

/// An order base, which `LOCAL_HUB` does not publish, with the values of [`order`].
pub fn order_base(symbol: &str) -> OrderBase {
    sample_order!(OrderBase(symbol) {})
}

/// An order with a distinct non-zero value in every field but the event flags. The market maker has up to 4
/// characters: the hub refuses a longer one.
pub fn order(symbol: &str) -> Order {
    sample_order!(Order(symbol) {
        market_maker: Some("MM1".to_owned()),
    })
}

/// An analytic order with a distinct non-zero value in every field but the event flags.
pub fn analytic_order(symbol: &str) -> AnalyticOrder {
    let mut order = sample_order!(AnalyticOrder(symbol) {
        market_maker: Some("MM2".to_owned()),
        iceberg_peak_size: 50.0,
        iceberg_hidden_size: 250.0,
        iceberg_executed_size: 25.0,
    });
    order.set_iceberg_type(IcebergType::Native);
    order
}

/// An OTC Markets order with a distinct non-zero value in every field but the event flags.
pub fn otc_markets_order(symbol: &str) -> OtcMarketsOrder {
    let mut order = sample_order!(OtcMarketsOrder(symbol) {
        market_maker: Some("MM3".to_owned()),
        quote_access_payment: 15,
    });
    order.set_open(true);
    order.set_otc_markets_price_type(OtcMarketsPriceType::Actual);
    order.set_auto_execution(true);
    order
}

/// A spread order with a distinct non-zero value in every field but the event flags.
pub fn spread_order(symbol: &str) -> SpreadOrder {
    sample_order!(SpreadOrder(symbol) {
        spread_symbol: Some("=AAPL-MSFT".to_owned()),
    })
}

/// A Nuam order with a distinct non-zero value in every field but the event flags.
pub fn nuam_order(symbol: &str) -> NuamOrder {
    let mut order = sample_order!(NuamOrder(symbol) {
        market_maker: Some("MM4".to_owned()),
        actor_id: 1,
        participant_id: 2,
        submitter_id: 3,
        on_behalf_of_submitter_id: 4,
        client_order_id: Some("cid".to_owned()),
        customer_account: Some("acct".to_owned()),
        customer_info: Some("cinfo".to_owned()),
        exchange_info: Some("einfo".to_owned()),
        time_in_force_data: 5,
        trigger_order_book_id: 6,
        trigger_price: 188.0,
        trigger_session_type: 7,
        order_quantity: 400.0,
        display_quantity: 100.0,
        refresh_quantity: 50.0,
        leaves_quantity: 200.0,
        matched_quantity: 100.0,
    });
    order.set_time_in_force(NuamTimeInForceType::Gtc);
    order.set_order_type(NuamOrderType::Limit);
    order
}

/// A market maker with a distinct non-zero value in every field but the event flags.
pub fn market_maker(symbol: &str) -> MarketMaker {
    let mut market_maker = MarketMaker {
        event_time: 1_700_000_000_500,
        bid_time: 1_700_000_000_000,
        bid_price: 189.5,
        bid_size: 100.0,
        bid_count: 3,
        ask_time: 1_700_000_001_000,
        ask_price: 189.75,
        ask_size: 200.0,
        ask_count: 4,
        ..MarketMaker::new(symbol)
    };
    market_maker.set_exchange_code('Q').unwrap();
    market_maker.set_market_maker("MM5").unwrap();
    market_maker
}

/// An order imbalance from the default source with a distinct non-zero value in every other field.
pub fn order_imbalance(symbol: &str) -> OrderImbalance {
    let mut imbalance = OrderImbalance {
        event_time: 1_700_000_000_500,
        time_sequence: INDEX,
        ref_price: 189.5,
        paired_size: 1000.0,
        imbalance_size: 250.0,
        near_price: 189.6,
        far_price: 189.4,
        ..OrderImbalance::new(symbol)
    };
    imbalance.set_imbalance_side(ImbalanceSide::Sell);
    imbalance.set_auction_type(AuctionType::Closing);
    imbalance
}

/// The order as `LOCAL_HUB` delivers it: the SDK does not carry through the hub the event time, the nanosecond part,
/// the action time, the order ids, the executed size and the trade of an order; they come back as zero or NaN.
macro_rules! delivered_order {
    ($type:ident($sent:expr) { $($field:ident: $value:expr,)* }) => {
        $type {
            event_time: 0,
            time_nano_part: 0,
            action_time: 0,
            order_id: 0,
            aux_order_id: 0,
            executed_size: f64::NAN,
            trade_id: 0,
            trade_price: f64::NAN,
            trade_size: f64::NAN,
            $($field: $value,)*
            ..$sent
        }
        .into()
    };
}

/// Whether the events are the same, NaN values included: their `Debug` texts are equal.
pub fn same_events<T: std::fmt::Debug + ?Sized>(a: &T, b: &T) -> bool {
    format!("{a:?}") == format!("{b:?}")
}

/// A configuration with a distinct non-zero value in every field.
pub fn configuration(symbol: &str) -> Configuration {
    Configuration {
        event_time: 1_700_000_000_500,
        version: 3,
        attachment: Some("{\"mode\": \"test\"}".to_owned()),
        ..Configuration::new(symbol)
    }
}

/// A message with a distinct non-zero value in every field.
pub fn message(symbol: &str) -> Message {
    Message {
        event_time: 1_700_000_000_500,
        attachment: Some("attach \u{e9}".to_owned()),
        ..Message::new(symbol)
    }
}

/// A text message with a distinct non-zero value in every field.
pub fn text_message(symbol: &str) -> TextMessage {
    TextMessage {
        event_time: 1_700_000_000_500,
        time_sequence: INDEX,
        text: Some("halt \u{1F600}".to_owned()),
        ..TextMessage::new(symbol)
    }
}

/// A text configuration with a distinct non-zero value in every field.
pub fn text_configuration(symbol: &str) -> TextConfiguration {
    TextConfiguration {
        event_time: 1_700_000_000_500,
        time_sequence: INDEX,
        version: 2,
        text: Some("mode=test".to_owned()),
        ..TextConfiguration::new(symbol)
    }
}

/// The candle symbol of the samples of candles for `symbol`: its daily candles.
pub fn candle_symbol(symbol: &str) -> String {
    format!("{symbol}{{=d}}")
}

/// One event of each mapped type, with the values of the samples; the candles are for [`candle_symbol`].
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
        time_and_sale(symbol).into(),
        nuam_time_and_sale(symbol).into(),
        option_sale(symbol).into(),
        candle(&candle_symbol(symbol)).into(),
        daily_candle(&candle_symbol(symbol)).into(),
        order(symbol).into(),
        analytic_order(symbol).into(),
        otc_markets_order(symbol).into(),
        spread_order(symbol).into(),
        nuam_order(symbol).into(),
        market_maker(symbol).into(),
        order_imbalance(symbol).into(),
        configuration(symbol).into(),
        message(symbol).into(),
        text_message(symbol).into(),
        text_configuration(symbol).into(),
    ]
}

/// The event as `LOCAL_HUB` delivers it: the SDK does not carry through the hub the event time of any event, the
/// millisecond-sequence of a quote, the nanosecond part of the trades and sales, the buyer and the seller of a time
/// and sale, the fields of [`delivered_order!`] and the iceberg of an analytic order, and the market maker of a Nuam
/// order; they come back as zero, NaN or `None`. Compare with [`same_events`]: NaN is not equal to itself.
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
        Event::TimeAndSale(e) => TimeAndSale {
            event_time: 0,
            time_nano_part: 0,
            buyer: None,
            seller: None,
            ..e
        }
        .into(),
        Event::NuamTimeAndSale(e) => NuamTimeAndSale {
            event_time: 0,
            time_nano_part: 0,
            buyer: None,
            seller: None,
            ..e
        }
        .into(),
        Event::OptionSale(e) => OptionSale {
            event_time: 0,
            time_nano_part: 0,
            ..e
        }
        .into(),
        Event::Candle(e) => Candle { event_time: 0, ..e }.into(),
        Event::DailyCandle(e) => DailyCandle { event_time: 0, ..e }.into(),
        Event::Order(e) => delivered_order!(Order(e) {}),
        Event::AnalyticOrder(e) => delivered_order!(AnalyticOrder(e) {
            iceberg_peak_size: f64::NAN,
            iceberg_hidden_size: f64::NAN,
            iceberg_executed_size: f64::NAN,
            iceberg_flags: 0,
        }),
        Event::OtcMarketsOrder(e) => delivered_order!(OtcMarketsOrder(e) {}),
        Event::SpreadOrder(e) => delivered_order!(SpreadOrder(e) {}),
        Event::NuamOrder(e) => delivered_order!(NuamOrder(e) {
            market_maker: None,
        }),
        Event::MarketMaker(e) => MarketMaker { event_time: 0, ..e }.into(),
        Event::OrderImbalance(e) => OrderImbalance { event_time: 0, ..e }.into(),
        Event::Configuration(e) => Configuration { event_time: 0, ..e }.into(),
        Event::Message(e) => Message { event_time: 0, ..e }.into(),
        Event::TextMessage(e) => TextMessage { event_time: 0, ..e }.into(),
        Event::TextConfiguration(e) => TextConfiguration { event_time: 0, ..e }.into(),
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
