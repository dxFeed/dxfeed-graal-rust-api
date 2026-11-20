//! The `LOCAL_HUB` fixture: events published to a hub reach its listeners on the test thread.

#![allow(dead_code)] // Each test binary uses a part.

use std::future::{Future, IntoFuture};
use std::io::Read;
use std::pin::{Pin, pin};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::task::{Context, Poll, Wake, Waker};
use std::thread::{self, Thread};
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

/// A waker that counts its calls and unparks the thread that created it.
pub struct CountingWaker {
    thread: Thread,
    wakes: AtomicUsize,
    wakes_by_ref: AtomicUsize,
}

impl CountingWaker {
    /// A waker of the current thread.
    pub fn new() -> Arc<Self> {
        Arc::new(CountingWaker {
            thread: thread::current(),
            wakes: AtomicUsize::new(0),
            wakes_by_ref: AtomicUsize::new(0),
        })
    }

    /// The calls of `wake`.
    pub fn wakes(&self) -> usize {
        self.wakes.load(Ordering::SeqCst)
    }

    /// The calls of `wake_by_ref`.
    pub fn wakes_by_ref(&self) -> usize {
        self.wakes_by_ref.load(Ordering::SeqCst)
    }

    /// All calls: `wake` and `wake_by_ref`.
    pub fn total(&self) -> usize {
        self.wakes() + self.wakes_by_ref()
    }
}

impl Wake for CountingWaker {
    fn wake(self: Arc<Self>) {
        self.wakes.fetch_add(1, Ordering::SeqCst);
        self.thread.unpark();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.wakes_by_ref.fetch_add(1, Ordering::SeqCst);
        self.thread.unpark();
    }
}

/// Polls `future` once with `waker`.
pub fn poll_with<F: Future + Unpin>(future: &mut F, waker: &Arc<CountingWaker>) -> Poll<F::Output> {
    let waker = Waker::from(Arc::clone(waker));
    Pin::new(future).poll(&mut Context::from_waker(&waker))
}

/// Runs `future` on this thread with `waker` until it is ready; `None` if it is not within `timeout`.
pub fn block_on_with<F: IntoFuture>(future: F, waker: &Arc<CountingWaker>, timeout: Duration) -> Option<F::Output> {
    let mut future = pin!(future.into_future());
    let deadline = Instant::now() + timeout;
    let waker = Waker::from(Arc::clone(waker));
    let mut context = Context::from_waker(&waker);
    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
            return Some(output);
        }
        let now = Instant::now();
        if now >= deadline {
            return None;
        }
        // A wake unparks the thread; a spurious unpark only polls again.
        thread::park_timeout(deadline - now);
    }
}

/// Runs `future` on this thread until it is ready; panics if it is not within 10 seconds.
pub fn block_on<F: IntoFuture>(future: F) -> F::Output {
    block_on_with(future, &CountingWaker::new(), Duration::from_secs(10)).expect("the future is not ready within 10 s")
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
    let mut e = Quote::new(symbol);
    e.event_time = 1_700_000_000_500;
    e.time_millis_sequence = (123 << 22) | 7;
    e.time_nano_part = 456;
    e.bid_time = 1_700_000_000_000;
    e.bid_exchange_code = 'Q';
    e.bid_price = 189.5;
    e.bid_size = 100.0;
    e.ask_time = 1_700_000_001_000;
    e.ask_exchange_code = 'N';
    e.ask_price = 189.75;
    e.ask_size = 200.0;
    e
}

/// A trade with a distinct non-zero value in every field.
pub fn trade(symbol: &str) -> Trade {
    let mut e = Trade::new(symbol);
    e.event_time = 1_700_000_000_500;
    e.time_sequence = (1_700_000_000_000 << 22) | 9;
    e.time_nano_part = 321;
    e.exchange_code = 'Q';
    e.price = 190.25;
    e.change = 0.5;
    e.size = 300.0;
    e.day_id = 19_675;
    e.day_volume = 1_000_000.0;
    e.day_turnover = 190_000_000.0;
    e.flags = 4;
    e.trade_id = 42;
    e
}

/// A trade in extended trading hours with the values of [`trade`].
pub fn trade_eth(symbol: &str) -> TradeETH {
    let t = trade(symbol);
    {
        let mut e = TradeETH::new(symbol);
        e.event_time = t.event_time;
        e.time_sequence = t.time_sequence;
        e.time_nano_part = t.time_nano_part;
        e.exchange_code = t.exchange_code;
        e.price = t.price;
        e.change = t.change;
        e.size = t.size;
        e.day_id = t.day_id;
        e.day_volume = t.day_volume;
        e.day_turnover = t.day_turnover;
        e.flags = t.flags | 1;
        e.trade_id = t.trade_id;
        e
    }
}

/// A Nuam trade with the values of [`trade`] and distinct non-zero statistics.
pub fn nuam_trade(symbol: &str) -> NuamTrade {
    let t = trade(symbol);
    {
        let mut e = NuamTrade::new(symbol);
        e.event_time = t.event_time;
        e.time_sequence = t.time_sequence;
        e.time_nano_part = t.time_nano_part;
        e.exchange_code = t.exchange_code;
        e.price = t.price;
        e.change = t.change;
        e.size = t.size;
        e.day_id = t.day_id;
        e.day_volume = t.day_volume;
        e.day_turnover = t.day_turnover;
        e.flags = t.flags;
        e.trade_id = t.trade_id;
        e.trade_stat_time = 1_700_000_002_000;
        e.last_significant_price = 190.5;
        e.last_price_for_all = 190.75;
        e.number_of_trades = 12_345;
        e.vwap = 190.125;
        e
    }
}

/// A profile with a distinct non-zero value in every field.
pub fn profile(symbol: &str) -> Profile {
    let mut profile = Profile::new(symbol);
    profile.event_time = 1_700_000_000_500;
    profile.description = Some("Apple Inc.".to_owned());
    profile.status_reason = Some("news".to_owned());
    profile.halt_start_time = 1_700_000_000_000;
    profile.halt_end_time = 1_700_000_100_000;
    profile.high_limit_price = 200.0;
    profile.low_limit_price = 150.0;
    profile.high_52_week_price = 199.5;
    profile.low_52_week_price = 120.25;
    profile.beta = 1.25;
    profile.earnings_per_share = 6.5;
    profile.dividend_frequency = 4.0;
    profile.ex_dividend_amount = 0.24;
    profile.ex_dividend_day_id = 19_600;
    profile.shares = 15_500_000_000.0;
    profile.free_float = 15_400_000_000.0;
    profile.set_short_sale_restriction(ShortSaleRestriction::Active);
    profile.set_trading_status(TradingStatus::Halted);
    profile
}

/// A summary with a distinct non-zero value in every field.
pub fn summary(symbol: &str) -> Summary {
    let mut summary = Summary::new(symbol);
    summary.event_time = 1_700_000_000_500;
    summary.day_id = 19_675;
    summary.day_open_price = 189.0;
    summary.day_high_price = 191.5;
    summary.day_low_price = 188.25;
    summary.day_close_price = 190.0;
    summary.prev_day_id = 19_674;
    summary.prev_day_close_price = 188.5;
    summary.prev_day_volume = 55_000_000.0;
    summary.open_interest = 1234;
    summary.set_day_close_price_type(PriceType::Final);
    summary.set_prev_day_close_price_type(PriceType::Preliminary);
    summary
}

/// The time and sequence of the samples of time series events: whole seconds and the sequence 5.
const INDEX: i64 = (1_700_000_000 << 32) | 5;

/// Greeks with a distinct non-zero value in every field but the event flags.
pub fn greeks(symbol: &str) -> Greeks {
    let mut e = Greeks::new(symbol);
    e.event_time = 1_700_000_000_500;
    e.index = INDEX;
    e.price = 12.5;
    e.volatility = 0.3;
    e.delta = 0.55;
    e.gamma = 0.02;
    e.theta = -0.05;
    e.rho = 0.1;
    e.vega = 0.2;
    e
}

/// An underlying with a distinct non-zero value in every field but the event flags.
pub fn underlying(symbol: &str) -> Underlying {
    let mut e = Underlying::new(symbol);
    e.event_time = 1_700_000_000_500;
    e.index = INDEX;
    e.volatility = 0.25;
    e.front_volatility = 0.24;
    e.back_volatility = 0.26;
    e.call_volume = 1000.0;
    e.put_volume = 800.0;
    e.put_call_ratio = 0.8;
    e
}

/// A theoretical price with a distinct non-zero value in every field but the event flags.
pub fn theo_price(symbol: &str) -> TheoPrice {
    let mut e = TheoPrice::new(symbol);
    e.event_time = 1_700_000_000_500;
    e.index = INDEX;
    e.price = 12.4;
    e.underlying_price = 190.0;
    e.delta = 0.56;
    e.gamma = 0.03;
    e.dividend = 0.01;
    e.interest = 0.05;
    e
}

/// A series with a distinct non-zero value in every field but the event flags.
pub fn series(symbol: &str) -> Series {
    let mut e = Series::new(symbol);
    e.event_time = 1_700_000_000_500;
    e.index = 17;
    e.time_sequence = INDEX;
    e.expiration = 19_700;
    e.volatility = 0.27;
    e.call_volume = 1100.0;
    e.put_volume = 900.0;
    e.put_call_ratio = 0.81;
    e.forward_price = 190.5;
    e.dividend = 0.02;
    e.interest = 0.04;
    e
}

/// A time and sale with a distinct non-zero value in every field but the event flags.
pub fn time_and_sale(symbol: &str) -> TimeAndSale {
    let mut sale = TimeAndSale::new(symbol);
    sale.event_time = 1_700_000_000_500;
    sale.index = INDEX;
    sale.time_nano_part = 7;
    sale.exchange_code = 'Q';
    sale.price = 190.5;
    sale.size = 300.0;
    sale.bid_price = 190.25;
    sale.ask_price = 190.75;
    sale.exchange_sale_conditions = Some("F01".to_owned());
    sale.buyer = Some("buyer_1".to_owned());
    sale.seller = Some("seller_1".to_owned());
    sale.trade_id = 4242;
    sale.set_trade_through_exempt('X').unwrap();
    sale.set_aggressor_side(Side::Buy);
    sale.set_valid_tick(true);
    sale.set_time_and_sale_type(TimeAndSaleType::Correction);
    sale
}

/// A Nuam time and sale with a distinct non-zero value in every field but the event flags.
pub fn nuam_time_and_sale(symbol: &str) -> NuamTimeAndSale {
    let sale = time_and_sale(symbol);
    {
        let mut e = NuamTimeAndSale::new(symbol);
        e.event_time = sale.event_time;
        e.index = sale.index;
        e.time_nano_part = sale.time_nano_part;
        e.exchange_code = sale.exchange_code;
        e.price = sale.price;
        e.size = sale.size;
        e.bid_price = sale.bid_price;
        e.ask_price = sale.ask_price;
        e.exchange_sale_conditions = sale.exchange_sale_conditions;
        e.flags = sale.flags;
        e.trade_id = sale.trade_id;
        e.buyer = sale.buyer;
        e.seller = sale.seller;
        e.match_id = 99;
        e
    }
}

/// An option sale with a distinct non-zero value in every field but the event flags.
pub fn option_sale(symbol: &str) -> OptionSale {
    let mut sale = OptionSale::new(symbol);
    sale.event_time = 1_700_000_000_500;
    sale.index = 17;
    sale.time_sequence = INDEX;
    sale.time_nano_part = 7;
    sale.exchange_code = 'C';
    sale.price = 2.5;
    sale.size = 10.0;
    sale.bid_price = 2.45;
    sale.ask_price = 2.55;
    sale.exchange_sale_conditions = Some("F02".to_owned());
    sale.underlying_price = 190.0;
    sale.volatility = 0.3;
    sale.delta = 0.45;
    sale.option_symbol = Some(".AAPL250117C150".to_owned());
    sale.set_aggressor_side(Side::Sell);
    sale.set_spread_leg(true);
    sale
}

/// A candle with a distinct non-zero value in every field but the event flags.
pub fn candle(symbol: &str) -> Candle {
    let mut e = Candle::new(symbol);
    e.event_time = 1_700_000_000_500;
    e.index = INDEX;
    e.count = 120;
    e.open = 189.0;
    e.high = 191.0;
    e.low = 188.5;
    e.close = 190.5;
    e.volume = 50_000.0;
    e.vwap = 190.1;
    e.bid_volume = 20_000.0;
    e.ask_volume = 30_000.0;
    e.imp_volatility = 0.28;
    e.open_interest = 7000.0;
    e
}

/// A daily candle with the values of [`candle`].
pub fn daily_candle(symbol: &str) -> DailyCandle {
    let candle = candle(symbol);
    {
        let mut e = DailyCandle::new(symbol);
        e.event_time = candle.event_time;
        e.index = candle.index;
        e.count = candle.count;
        e.open = candle.open;
        e.high = candle.high;
        e.low = candle.low;
        e.close = candle.close;
        e.volume = candle.volume;
        e.vwap = candle.vwap;
        e.bid_volume = candle.bid_volume;
        e.ask_volume = candle.ask_volume;
        e.imp_volatility = candle.imp_volatility;
        e.open_interest = candle.open_interest;
        e
    }
}

/// An order of type `$type` from the default source with a distinct non-zero value in every field of `OrderBase` but
/// the event flags, and the fields given.
macro_rules! sample_order {
    ($type:ident($symbol:expr) { $($field:ident: $value:expr,)* }) => {{
        let mut order = $type::new($symbol);
        order.event_time = 1_700_000_000_500;
        order.index = 17;
        order.time_sequence = INDEX;
        order.time_nano_part = 7;
        order.action_time = 1_700_000_000_400;
        order.order_id = 1001;
        order.aux_order_id = 1002;
        order.price = 189.5;
        order.size = 300.0;
        order.executed_size = 100.0;
        order.count = 4;
        order.trade_id = 42;
        order.trade_price = 189.25;
        order.trade_size = 50.0;
        $(order.$field = $value;)*
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
    let mut market_maker = MarketMaker::new(symbol);
    market_maker.event_time = 1_700_000_000_500;
    market_maker.bid_time = 1_700_000_000_000;
    market_maker.bid_price = 189.5;
    market_maker.bid_size = 100.0;
    market_maker.bid_count = 3;
    market_maker.ask_time = 1_700_000_001_000;
    market_maker.ask_price = 189.75;
    market_maker.ask_size = 200.0;
    market_maker.ask_count = 4;
    market_maker.set_exchange_code('Q').unwrap();
    market_maker.set_market_maker("MM5").unwrap();
    market_maker
}

/// An order imbalance from the default source with a distinct non-zero value in every other field.
pub fn order_imbalance(symbol: &str) -> OrderImbalance {
    let mut imbalance = OrderImbalance::new(symbol);
    imbalance.event_time = 1_700_000_000_500;
    imbalance.time_sequence = INDEX;
    imbalance.ref_price = 189.5;
    imbalance.paired_size = 1000.0;
    imbalance.imbalance_size = 250.0;
    imbalance.near_price = 189.6;
    imbalance.far_price = 189.4;
    imbalance.set_imbalance_side(ImbalanceSide::Sell);
    imbalance.set_auction_type(AuctionType::Closing);
    imbalance
}

/// The order as `LOCAL_HUB` delivers it: the SDK does not carry through the hub the event time, the nanosecond part,
/// the action time, the order ids, the executed size and the trade of an order; they come back as zero or NaN.
macro_rules! delivered_order {
    ($type:ident($sent:expr) { $($field:ident: $value:expr,)* }) => {{
        let mut order: $type = $sent;
        order.event_time = 0;
        order.time_nano_part = 0;
        order.action_time = 0;
        order.order_id = 0;
        order.aux_order_id = 0;
        order.executed_size = f64::NAN;
        order.trade_id = 0;
        order.trade_price = f64::NAN;
        order.trade_size = f64::NAN;
        $(order.$field = $value;)*
        order.into()
    }};
}

/// Whether the events are the same, NaN values included: their `Debug` texts are equal.
pub fn same_events<T: std::fmt::Debug + ?Sized>(a: &T, b: &T) -> bool {
    format!("{a:?}") == format!("{b:?}")
}

/// A configuration with a distinct non-zero value in every field.
pub fn configuration(symbol: &str) -> Configuration {
    let mut e = Configuration::new(symbol);
    e.event_time = 1_700_000_000_500;
    e.version = 3;
    e.attachment = Some("{\"mode\": \"test\"}".to_owned());
    e
}

/// A message with a distinct non-zero value in every field.
pub fn message(symbol: &str) -> Message {
    let mut e = Message::new(symbol);
    e.event_time = 1_700_000_000_500;
    e.attachment = Some("attach \u{e9}".to_owned());
    e
}

/// A text message with a distinct non-zero value in every field.
pub fn text_message(symbol: &str) -> TextMessage {
    let mut e = TextMessage::new(symbol);
    e.event_time = 1_700_000_000_500;
    e.time_sequence = INDEX;
    e.text = Some("halt \u{1F600}".to_owned());
    e
}

/// A text configuration with a distinct non-zero value in every field.
pub fn text_configuration(symbol: &str) -> TextConfiguration {
    let mut e = TextConfiguration::new(symbol);
    e.event_time = 1_700_000_000_500;
    e.time_sequence = INDEX;
    e.version = 2;
    e.text = Some("mode=test".to_owned());
    e
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
        Event::Quote(mut e) => {
            e.event_time = 0;
            e.time_millis_sequence = 0;
            e.time_nano_part = 0;
            e.into()
        }
        Event::Profile(mut e) => {
            e.event_time = 0;
            e.into()
        }
        Event::Summary(mut e) => {
            e.event_time = 0;
            e.into()
        }
        Event::Greeks(mut e) => {
            e.event_time = 0;
            e.into()
        }
        Event::Underlying(mut e) => {
            e.event_time = 0;
            e.into()
        }
        Event::TheoPrice(mut e) => {
            e.event_time = 0;
            e.into()
        }
        Event::Trade(mut e) => {
            e.event_time = 0;
            e.time_nano_part = 0;
            e.into()
        }
        Event::TradeETH(mut e) => {
            e.event_time = 0;
            e.time_nano_part = 0;
            e.into()
        }
        Event::Series(mut e) => {
            e.event_time = 0;
            e.into()
        }
        Event::NuamTrade(mut e) => {
            e.event_time = 0;
            e.time_nano_part = 0;
            e.into()
        }
        Event::TimeAndSale(mut e) => {
            e.event_time = 0;
            e.time_nano_part = 0;
            e.buyer = None;
            e.seller = None;
            e.into()
        }
        Event::NuamTimeAndSale(mut e) => {
            e.event_time = 0;
            e.time_nano_part = 0;
            e.buyer = None;
            e.seller = None;
            e.into()
        }
        Event::OptionSale(mut e) => {
            e.event_time = 0;
            e.time_nano_part = 0;
            e.into()
        }
        Event::Candle(mut e) => {
            e.event_time = 0;
            e.into()
        }
        Event::DailyCandle(mut e) => {
            e.event_time = 0;
            e.into()
        }
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
        Event::MarketMaker(mut e) => {
            e.event_time = 0;
            e.into()
        }
        Event::OrderImbalance(mut e) => {
            e.event_time = 0;
            e.into()
        }
        Event::Configuration(mut e) => {
            e.event_time = 0;
            e.into()
        }
        Event::Message(mut e) => {
            e.event_time = 0;
            e.into()
        }
        Event::TextMessage(mut e) => {
            e.event_time = 0;
            e.into()
        }
        Event::TextConfiguration(mut e) => {
            e.event_time = 0;
            e.into()
        }
        other => panic!("no delivered form of {other}"),
    }
}

/// [`quote`] with another bid price: the hub does not deliver a quote identical to the last one.
pub fn quote_with_bid(symbol: &str, bid_price: f64) -> Quote {
    let mut e = quote(symbol);
    e.bid_price = bid_price;
    e
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
