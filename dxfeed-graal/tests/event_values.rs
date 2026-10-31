//! The Java constants and conversions around the event types: `values()` and `valueOf` of the event enums,
//! `MAX_SEQUENCE`, the no-argument constructors and the integer sizes. Offline, no network needed.

use std::fmt::Display;
use std::str::FromStr;

use dxfeed_graal::Error;
use dxfeed_graal::event::{
    AnalyticOrder, AuctionType, Candle, DailyCandle, Direction, Greeks, IcebergType, ImbalanceSide, Message, NuamOrder,
    NuamOrderType, NuamTimeAndSale, NuamTimeInForceType, NuamTrade, OptionSale, Order, OrderAction, OrderBase,
    OrderImbalance, OtcMarketsOrder, OtcMarketsPriceType, PriceType, Quote, Scope, Series, ShortSaleRestriction, Side,
    SpreadOrder, TextConfiguration, TextMessage, TheoPrice, TimeAndSale, TimeAndSaleType, Trade, TradeETH,
    TradingStatus, Underlying,
};

/// Checks that `E::ALL` has the `count` values of Java's `values()`, that each parses back from its Java name and
/// that other names are refused.
fn check_enum<E>(all: &[E], count: usize)
where
    E: Copy + PartialEq + std::fmt::Debug + Display + FromStr<Err = Error>,
{
    assert_eq!(all.len(), count, "{all:?}");
    for &value in all {
        let name = value.to_string();
        assert_eq!(name.parse::<E>().unwrap(), value, "{name}");
        assert!(name.to_lowercase().parse::<E>().is_err(), "{name}");
    }
    assert!(matches!("UNKNOWN_VALUE".parse::<E>(), Err(Error::InvalidArgument(_))));
    assert!("".parse::<E>().is_err());
}

#[test]
fn enums_have_the_values_of_java() {
    check_enum(Direction::ALL, 6);
    check_enum(TradingStatus::ALL, 3);
    check_enum(ShortSaleRestriction::ALL, 3);
    check_enum(Side::ALL, 3);
    check_enum(Scope::ALL, 4);
    check_enum(OrderAction::ALL, 9);
    check_enum(IcebergType::ALL, 3);
    check_enum(OtcMarketsPriceType::ALL, 3);
    check_enum(NuamOrderType::ALL, 6);
    check_enum(NuamTimeInForceType::ALL, 7);
    check_enum(ImbalanceSide::ALL, 6);
    check_enum(AuctionType::ALL, 5);
    check_enum(TimeAndSaleType::ALL, 3);
    check_enum(PriceType::ALL, 4);
}

#[test]
fn enums_parse_java_names() {
    assert_eq!("ZERO_UP".parse::<Direction>().unwrap(), Direction::ZeroUp);
    assert_eq!("BUY".parse::<Side>().unwrap(), Side::Buy);
    assert_eq!("GTC".parse::<NuamTimeInForceType>().unwrap(), NuamTimeInForceType::Gtc);
    assert_eq!(Direction::ALL[0], Direction::Undefined);
    assert_eq!(Scope::ALL[3], Scope::Order);
}

#[test]
fn max_sequence_is_that_of_java() {
    const JAVA: i32 = (1 << 22) - 1;
    let all = [
        Candle::MAX_SEQUENCE,
        DailyCandle::MAX_SEQUENCE,
        OptionSale::MAX_SEQUENCE,
        OrderBase::MAX_SEQUENCE,
        Order::MAX_SEQUENCE,
        AnalyticOrder::MAX_SEQUENCE,
        OtcMarketsOrder::MAX_SEQUENCE,
        SpreadOrder::MAX_SEQUENCE,
        NuamOrder::MAX_SEQUENCE,
        OrderImbalance::MAX_SEQUENCE,
        Quote::MAX_SEQUENCE,
        TimeAndSale::MAX_SEQUENCE,
        NuamTimeAndSale::MAX_SEQUENCE,
        Trade::MAX_SEQUENCE,
        TradeETH::MAX_SEQUENCE,
        NuamTrade::MAX_SEQUENCE,
        TextConfiguration::MAX_SEQUENCE,
        TextMessage::MAX_SEQUENCE,
        Greeks::MAX_SEQUENCE,
        Series::MAX_SEQUENCE,
        TheoPrice::MAX_SEQUENCE,
        Underlying::MAX_SEQUENCE,
    ];
    assert!(all.iter().all(|&max| max == JAVA), "{all:?}");

    let mut quote = Quote::new("AAPL");
    quote.set_sequence(Quote::MAX_SEQUENCE).unwrap();
    assert_eq!(quote.sequence(), Quote::MAX_SEQUENCE);
    assert!(quote.set_sequence(Quote::MAX_SEQUENCE + 1).is_err());
    let mut order = Order::new("AAPL");
    assert!(order.set_sequence(Order::MAX_SEQUENCE + 1).is_err());
}

#[test]
fn default_is_the_no_argument_constructor() {
    // The NaN fields make `==` false, so the texts are compared.
    let same = |a: &dyn std::fmt::Debug, b: &dyn std::fmt::Debug| format!("{a:?}") == format!("{b:?}");
    assert!(same(&Quote::default(), &Quote::new("")));
    assert!(same(&Order::default(), &Order::new("")));
    assert!(same(&Candle::default(), &Candle::new("")));
    assert_eq!(Message::default().symbol, "");
    assert!(TimeAndSale::default().price.is_nan());
}

#[test]
fn integer_sizes_drop_the_fraction() {
    let mut quote = Quote::new("AAPL");
    assert_eq!(quote.bid_size_as_long(), 0);
    quote.bid_size = 12.7;
    quote.ask_size = -3.9;
    assert_eq!(quote.bid_size_as_long(), 12);
    assert_eq!(quote.ask_size_as_long(), -3);

    let mut order = Order::new("AAPL");
    order.size = 1e20;
    assert_eq!(order.size_as_long(), i64::MAX);

    let mut sale = TimeAndSale::new("AAPL");
    sale.size = 100.5;
    assert_eq!(sale.size_as_long(), 100);

    let mut trade = Trade::new("AAPL");
    trade.size = 7.0;
    trade.day_volume = 1_000.9;
    assert_eq!((trade.size_as_long(), trade.day_volume_as_long()), (7, 1_000));

    assert_eq!(TradeETH::new("AAPL").day_volume_as_long(), 0);
    let mut daily = DailyCandle::new("AAPL");
    daily.open_interest = 42.9;
    assert_eq!((daily.open_interest_as_long(), daily.volume_as_long()), (42, 0));

    let mut candle = Candle::new("AAPL");
    candle.volume = 5.5;
    candle.bid_volume = 2.2;
    assert_eq!(
        (
            candle.volume_as_long(),
            candle.bid_volume_as_long(),
            candle.ask_volume_as_long()
        ),
        (5, 2, 0)
    );
}
