//! `Display` of events: the text of Java's `toString`. The expected strings are written from the Java formats.

mod support;

use std::sync::mpsc;

use dxfeed_graal::event::{
    AnalyticOrder, AuctionType, Candle, DailyCandle, Direction, EventFlags, Greeks, IcebergType, ImbalanceSide,
    IndexedEvent, MarketEvent, MarketMaker, NuamOrder, NuamOrderType, NuamTimeAndSale, NuamTimeInForceType, NuamTrade,
    OptionSale, Order, OrderAction, OrderBase, OrderImbalance, OtcMarketsOrder, OtcMarketsPriceType, PriceType,
    Profile, Quote, Scope, Series, ShortSaleRestriction, Side, SpreadOrder, Summary, TheoPrice, TimeAndSale,
    TimeAndSaleType, TimeSeriesEvent, Trade, TradeETH, TradingStatus, Underlying,
};
use dxfeed_graal::{Error, Event, EventRef, EventType};
use support::{Hub, is_time, is_time_without_millis, quote};

/// The value of `name=` in the text of an event.
fn field<'a>(text: &'a str, name: &str) -> &'a str {
    let start = text
        .find(&format!(", {name}="))
        .unwrap_or_else(|| panic!("no {name} in {text}"))
        + name.len()
        + 3;
    let rest = &text[start..];
    &rest[..rest.find([',', '}']).unwrap()]
}

#[test]
fn new_quote_prints_as_in_java() {
    assert_eq!(
        Quote::new("AAPL").to_string(),
        "Quote{AAPL, eventTime=0, time=0, timeNanoPart=0, sequence=0, bidTime=0, bidExchange=\\0, bidPrice=NaN, \
         bidSize=NaN, askTime=0, askExchange=\\0, askPrice=NaN, askSize=NaN}"
    );
}

#[test]
fn new_trade_prints_as_in_java() {
    assert_eq!(
        Trade::new("AAPL").to_string(),
        "Trade{AAPL, eventTime=0, time=0, timeNanoPart=0, sequence=0, exchange=\\0, tradeId=0, price=NaN, change=NaN, \
         size=NaN, day=19700101, dayVolume=NaN, dayTurnover=NaN, direction=UNDEFINED, ETH=false}"
    );
}

/// Strings printed by Java 3.356 for events filled through the Java setters of the same names.
#[test]
fn typed_setters_print_as_in_java() {
    let mut quote = Quote {
        bid_exchange_code: 'Q',
        bid_price: 189.5,
        bid_size: 100.0,
        ask_exchange_code: 'N',
        ask_price: 189.75,
        ask_size: 200.0,
        time_nano_part: 456,
        ..Quote::new("AAPL")
    };
    quote.set_sequence(7).unwrap();
    assert_eq!(
        quote.to_string(),
        "Quote{AAPL, eventTime=0, time=0, timeNanoPart=456, sequence=7, bidTime=0, bidExchange=Q, bidPrice=189.5, \
         bidSize=100.0, askTime=0, askExchange=N, askPrice=189.75, askSize=200.0}"
    );

    let base = || {
        let mut trade = Trade {
            exchange_code: 'Q',
            price: 190.25,
            change: 0.5,
            size: 300.0,
            day_id: 19675,
            day_volume: 1_000_000.0,
            day_turnover: 1.9e8,
            time_nano_part: 321,
            trade_id: 42,
            ..Trade::new("AAPL")
        };
        trade.set_sequence(9).unwrap();
        trade
    };
    let prefix = "Trade{AAPL, eventTime=0, time=0, timeNanoPart=321, sequence=9, exchange=Q, tradeId=42, price=190.25, \
                  change=0.5, size=300.0, day=20231114, dayVolume=1000000.0, dayTurnover=1.9E8, ";
    let mut eth = base();
    eth.set_extended_trading_hours(true);
    assert_eq!(
        (eth.flags, eth.to_string()),
        (1, format!("{prefix}direction=UNDEFINED, ETH=true}}"))
    );
    let mut zero_down = base();
    zero_down.set_tick_direction(Direction::ZeroDown);
    assert_eq!(
        (zero_down.flags, zero_down.to_string()),
        (4, format!("{prefix}direction=ZERO_DOWN, ETH=false}}"))
    );
    let mut zero_up = base();
    zero_up.set_tick_direction(Direction::ZeroUp);
    zero_up.set_extended_trading_hours(true);
    assert_eq!(
        (zero_up.flags, zero_up.to_string()),
        (9, format!("{prefix}direction=ZERO_UP, ETH=true}}"))
    );
    assert_eq!(zero_up.tick_direction(), Direction::ZeroUp);
    assert!(zero_up.is_extended_trading_hours());
}

#[test]
fn setters_keep_the_other_packed_values() {
    let mut quote = Quote::new("AAPL");
    quote.set_sequence(5).unwrap();
    quote.set_bid_time(1_700_000_000_123);
    quote.set_ask_time(1_700_000_000_045);
    assert_eq!((quote.time(), quote.sequence()), (1_700_000_000_123, 5));
    assert_eq!(quote.time_nanos(), 1_700_000_000_123_000_000);
    assert!(matches!(quote.set_sequence(1 << 22), Err(Error::InvalidArgument(_))));
    assert!(matches!(quote.set_sequence(-1), Err(Error::InvalidArgument(_))));
    assert_eq!(quote.sequence(), 5);

    let mut trade = Trade::new("AAPL");
    trade.set_sequence(3).unwrap();
    trade.set_time_nanos(1_700_000_000_789_000_456);
    trade.set_tick_direction(Direction::Up);
    trade.set_extended_trading_hours(true);
    trade.set_tick_direction(Direction::Down);
    assert_eq!(trade.time(), 1_700_000_000_789);
    assert_eq!(trade.time_nanos(), 1_700_000_000_789_000_456);
    assert_eq!((trade.time_nano_part, trade.sequence()), (456, 3));
    assert_eq!(
        (trade.tick_direction(), trade.is_extended_trading_hours()),
        (Direction::Down, true)
    );
    trade.set_extended_trading_hours(false);
    assert_eq!(trade.flags, Direction::Down.code() << 1);
}

#[test]
fn events_convert_back_to_their_type() {
    // `quote` has no NaN, which would make the events unequal to themselves.
    let event = Event::from(quote("AAPL"));
    assert_eq!(Trade::try_from(event.clone()), Err(event.clone()));
    assert_eq!(Quote::try_from(event), Ok(quote("AAPL")));
}

#[test]
fn quote_values_print_as_in_java() {
    let quote = Quote {
        time_millis_sequence: 5,
        time_nano_part: 7,
        bid_exchange_code: 'Q',
        bid_price: 100.0,
        bid_size: 1e7,
        ask_exchange_code: 'K',
        ask_price: 101.5,
        ask_size: 250.0,
        ..Quote::new("IBM")
    };
    assert_eq!(
        quote.to_string(),
        "Quote{IBM, eventTime=0, time=0, timeNanoPart=7, sequence=5, bidTime=0, bidExchange=Q, bidPrice=100.0, \
         bidSize=1.0E7, askTime=0, askExchange=K, askPrice=101.5, askSize=250.0}"
    );
}

#[test]
fn trade_values_print_as_in_java() {
    let trade = Trade {
        time_sequence: 9,
        exchange_code: '\u{e9}',
        price: 1.5e-5,
        change: -0.25,
        size: 12345678.9,
        day_id: 20000,
        day_volume: 0.0001,
        day_turnover: 1e21,
        flags: 4 << 1 | 1,
        trade_id: 42,
        ..Trade::new("MSFT")
    };
    assert_eq!(
        trade.to_string(),
        "Trade{MSFT, eventTime=0, time=0, timeNanoPart=0, sequence=9, exchange=\\u00e9, tradeId=42, price=1.5E-5, \
         change=-0.25, size=1.23456789E7, day=20241004, dayVolume=1.0E-4, dayTurnover=1.0E21, direction=ZERO_UP, \
         ETH=true}"
    );
}

#[test]
fn times_print_through_the_sdk() {
    let quote = Quote {
        event_time: 1_700_000_000_123,
        time_millis_sequence: 456 << 22,
        bid_time: 1_700_000_000_000,
        ask_time: 1_700_000_001_000,
        ..Quote::new("AAPL")
    };
    let text = quote.to_string();
    for name in ["eventTime", "time", "bidTime", "askTime"] {
        assert!(is_time(field(&text, name)), "{name} in {text}");
    }
    assert!(field(&text, "eventTime").ends_with(".123"), "{text}");
    assert!(field(&text, "time").ends_with(".456"), "{text}");
    assert!(field(&text, "bidTime").ends_with(".000"), "{text}");
    // Both times fall on one minute in every time zone: only the seconds differ.
    assert_eq!(field(&text, "bidTime")[..13], field(&text, "askTime")[..13]);

    let trade = Trade {
        time_sequence: (1_700_000_000_i64 << 32) | (789 << 22),
        ..Trade::new("AAPL")
    };
    let text = trade.to_string();
    assert!(is_time(field(&text, "time")), "{text}");
    assert!(field(&text, "time").ends_with(".789"), "{text}");
}

#[test]
fn events_print_as_their_payload() {
    let quote = Quote {
        bid_price: 1.0,
        ..Quote::new("AAPL")
    };
    assert_eq!(Event::from(quote.clone()).to_string(), quote.to_string());
    assert_eq!(
        Event::from(Trade::new("AAPL")).to_string(),
        Trade::new("AAPL").to_string()
    );
    assert_eq!(EventRef::Other(42).to_string(), "Event{class=42}");
}

#[test]
fn received_events_print_like_their_copies() {
    let hub = Hub::new();
    let subscription = hub.feed.create_subscription::<Quote>().unwrap();
    let (sender, texts) = mpsc::channel();
    subscription
        .add_event_listener(move |quotes| {
            for quote in quotes {
                // Printing a time calls the SDK from inside the listener.
                let _ = sender.send((quote.to_string(), quote.to_owned().to_string()));
            }
        })
        .unwrap();
    subscription.add_symbols(["AAPL"]).unwrap();
    let sent = Quote {
        bid_time: 1_700_000_000_000,
        ..quote("AAPL")
    };
    hub.publish_and_process(&[sent.into()]);
    let (borrowed, owned) = texts.try_recv().expect("no quote");
    assert_eq!(borrowed, owned);
    assert!(is_time(field(&borrowed, "bidTime")), "{borrowed}");

    let untyped = hub.feed.create_subscription_with_types(&[EventType::Quote]).unwrap();
    let (sender, texts) = mpsc::channel();
    untyped
        .add_event_listener(move |events| {
            for event in events {
                let _ = sender.send((event.to_string(), event.to_owned().map(|e| e.to_string())));
            }
        })
        .unwrap();
    untyped.add_symbols(["IBM"]).unwrap();
    hub.publish_and_process(&[quote("IBM").into()]);
    let (borrowed, owned) = texts.try_recv().expect("no event");
    assert_eq!(Some(borrowed), owned);
}

/// The strings Java 3.356 prints for `new Xxx("AAPL")`.
#[test]
fn new_market_events_print_as_in_java() {
    let cases: [(String, &str); 8] = [
        (
            Profile::new("AAPL").to_string(),
            "Profile{AAPL, eventTime=0, description='null', SSR=UNDEFINED, status=UNDEFINED, statusReason='null', \
             haltStartTime=0, haltEndTime=0, highLimitPrice=NaN, lowLimitPrice=NaN, high52WeekPrice=NaN, \
             low52WeekPrice=NaN, beta=NaN, earningsPerShare=NaN, dividendFrequency=NaN, exDividendAmount=NaN, \
             exDividendDay=19700101, shares=NaN, freeFloat=NaN}",
        ),
        (
            Summary::new("AAPL").to_string(),
            "Summary{AAPL, eventTime=0, day=19700101, dayOpen=NaN, dayHigh=NaN, dayLow=NaN, dayClose=NaN, \
             dayCloseType=REGULAR, prevDay=19700101, prevDayClose=NaN, prevDayCloseType=REGULAR, prevDayVolume=NaN, \
             openInterest=0}",
        ),
        (
            Greeks::new("AAPL").to_string(),
            "Greeks{AAPL, eventTime=0, eventFlags=0x0, time=0, sequence=0, price=NaN, volatility=NaN, delta=NaN, \
             gamma=NaN, theta=NaN, rho=NaN, vega=NaN}",
        ),
        (
            Underlying::new("AAPL").to_string(),
            "Underlying{AAPL, eventTime=0, eventFlags=0x0, time=0, sequence=0, volatility=NaN, frontVolatility=NaN, \
             backVolatility=NaN, callVolume=NaN, putVolume=NaN, putCallRatio=NaN}",
        ),
        (
            TheoPrice::new("AAPL").to_string(),
            "TheoPrice{AAPL, eventTime=0, eventFlags=0x0, time=0, sequence=0, price=NaN, underlyingPrice=NaN, \
             delta=NaN, gamma=NaN, dividend=NaN, interest=NaN}",
        ),
        (
            TradeETH::new("AAPL").to_string(),
            "TradeETH{AAPL, eventTime=0, time=0, timeNanoPart=0, sequence=0, exchange=\\0, tradeId=0, price=NaN, \
             change=NaN, size=NaN, day=19700101, dayVolume=NaN, dayTurnover=NaN, direction=UNDEFINED, ETH=false}",
        ),
        (
            Series::new("AAPL").to_string(),
            "Series{AAPL, eventTime=0, eventFlags=0x0, index=0x0, time=0, sequence=0, expiration=19700101, \
             volatility=NaN, callVolume=NaN, putVolume=NaN, putCallRatio=NaN, forwardPrice=NaN, dividend=NaN, \
             interest=NaN}",
        ),
        (
            NuamTrade::new("AAPL").to_string(),
            "NuamTrade{AAPL, eventTime=0, time=0, timeNanoPart=0, sequence=0, exchange=\\0, tradeId=0, price=NaN, \
             change=NaN, size=NaN, day=19700101, dayVolume=NaN, dayTurnover=NaN, direction=UNDEFINED, ETH=false, \
             tradeStatTime=0, lastSignificantPrice=NaN, lastPriceForAll=NaN, numberOfTrades=0, vwap=NaN}",
        ),
    ];
    for (rust, java) in cases {
        assert_eq!(rust, java);
    }
}

/// The strings Java 3.356 prints for events filled through the Java setters of the same names.
#[test]
fn filled_market_events_print_as_in_java() {
    let mut profile = Profile {
        description: Some("desc".to_owned()),
        status_reason: Some("reason".to_owned()),
        high_limit_price: 10.0,
        low_limit_price: 1.5,
        high_52_week_price: 1e7,
        low_52_week_price: 2.25,
        beta: 0.75,
        earnings_per_share: 3.5,
        dividend_frequency: 4.0,
        ex_dividend_amount: 0.25,
        ex_dividend_day_id: 19000,
        shares: 1000.0,
        free_float: 500.5,
        ..Profile::new("AAPL")
    };
    profile.set_short_sale_restriction(ShortSaleRestriction::Active);
    profile.set_trading_status(TradingStatus::Active);
    assert_eq!(profile.flags, 6);
    assert_eq!(
        profile.to_string(),
        "Profile{AAPL, eventTime=0, description='desc', SSR=ACTIVE, status=ACTIVE, statusReason='reason', \
         haltStartTime=0, haltEndTime=0, highLimitPrice=10.0, lowLimitPrice=1.5, high52WeekPrice=1.0E7, \
         low52WeekPrice=2.25, beta=0.75, earningsPerShare=3.5, dividendFrequency=4.0, exDividendAmount=0.25, \
         exDividendDay=20220108, shares=1000, freeFloat=500.5}"
    );
    let empty = Profile {
        description: Some(String::new()),
        status_reason: Some(String::new()),
        ..Profile::new("AAPL")
    };
    assert!(
        empty
            .to_string()
            .contains("description='', SSR=UNDEFINED, status=UNDEFINED, statusReason='',"),
        "{empty}"
    );

    let mut summary = Summary {
        day_id: 19001,
        day_open_price: 1.5,
        day_high_price: 2.25,
        day_low_price: 1.0,
        day_close_price: 10.0,
        prev_day_id: 19000,
        prev_day_close_price: 1e7,
        prev_day_volume: 1234.5,
        open_interest: 77,
        ..Summary::new("AAPL")
    };
    summary.set_day_close_price_type(PriceType::Final);
    summary.set_prev_day_close_price_type(PriceType::Preliminary);
    assert_eq!(summary.flags, 14);
    assert_eq!(
        summary.to_string(),
        "Summary{AAPL, eventTime=0, day=20220109, dayOpen=1.5, dayHigh=2.25, dayLow=1.0, dayClose=10.0, \
         dayCloseType=FINAL, prevDay=20220108, prevDayClose=1.0E7, prevDayCloseType=PRELIMINARY, \
         prevDayVolume=1234.5, openInterest=77}"
    );

    let mut greeks = Greeks {
        price: 1.5,
        volatility: 0.25,
        delta: 0.5,
        gamma: 2.25,
        theta: -1.5,
        rho: 10.0,
        vega: 1e7,
        ..Greeks::new("AAPL")
    };
    greeks.set_sequence(5).unwrap();
    assert_eq!(greeks.index, 5);
    assert_eq!(
        greeks.to_string(),
        "Greeks{AAPL, eventTime=0, eventFlags=0x0, time=0, sequence=5, price=1.5, volatility=0.25, delta=0.5, \
         gamma=2.25, theta=-1.5, rho=10.0, vega=1.0E7}"
    );

    let mut underlying = Underlying {
        volatility: 0.25,
        front_volatility: 0.5,
        back_volatility: 0.75,
        call_volume: 1e7,
        put_volume: 50.0,
        put_call_ratio: 0.5,
        ..Underlying::new("AAPL")
    };
    underlying.set_sequence(5).unwrap();
    assert_eq!(
        underlying.to_string(),
        "Underlying{AAPL, eventTime=0, eventFlags=0x0, time=0, sequence=5, volatility=0.25, frontVolatility=0.5, \
         backVolatility=0.75, callVolume=1.0E7, putVolume=50.0, putCallRatio=0.5}"
    );

    let mut theo_price = TheoPrice {
        price: 1.5,
        underlying_price: 1e7,
        delta: 0.5,
        gamma: 2.25,
        dividend: 0.25,
        interest: 0.05,
        ..TheoPrice::new("AAPL")
    };
    theo_price.set_sequence(5).unwrap();
    assert_eq!(
        theo_price.to_string(),
        "TheoPrice{AAPL, eventTime=0, eventFlags=0x0, time=0, sequence=5, price=1.5, underlyingPrice=1.0E7, \
         delta=0.5, gamma=2.25, dividend=0.25, interest=0.05}"
    );

    let mut series = Series {
        index: 17,
        expiration: 19999,
        volatility: 0.25,
        call_volume: 100.0,
        put_volume: 50.0,
        put_call_ratio: 0.5,
        forward_price: 1e7,
        dividend: 0.25,
        interest: 0.05,
        ..Series::new("AAPL")
    };
    series.set_sequence(5).unwrap();
    assert_eq!(
        series.to_string(),
        "Series{AAPL, eventTime=0, eventFlags=0x0, index=0x11, time=0, sequence=5, expiration=20241003, \
         volatility=0.25, callVolume=100.0, putVolume=50.0, putCallRatio=0.5, forwardPrice=1.0E7, dividend=0.25, \
         interest=0.05}"
    );

    let trade_base = "AAPL, eventTime=0, time=0, timeNanoPart=7, sequence=5, exchange=Q, tradeId=42, price=1.5, \
                      change=2.25, size=10.0, day=20220108, dayVolume=1000.0, dayTurnover=1.0E7, direction=UP, \
                      ETH=true";
    let mut trade_eth = TradeETH {
        exchange_code: 'Q',
        price: 1.5,
        change: 2.25,
        size: 10.0,
        day_id: 19000,
        day_volume: 1000.0,
        day_turnover: 1e7,
        time_nano_part: 7,
        trade_id: 42,
        ..TradeETH::new("AAPL")
    };
    trade_eth.set_sequence(5).unwrap();
    trade_eth.set_tick_direction(Direction::Up);
    trade_eth.set_extended_trading_hours(true);
    assert_eq!(trade_eth.flags, 11);
    assert_eq!(trade_eth.to_string(), format!("TradeETH{{{trade_base}}}"));

    let mut nuam_trade = NuamTrade {
        exchange_code: 'Q',
        price: 1.5,
        change: 2.25,
        size: 10.0,
        day_id: 19000,
        day_volume: 1000.0,
        day_turnover: 1e7,
        time_nano_part: 7,
        trade_id: 42,
        last_significant_price: 1.5,
        last_price_for_all: 2.25,
        number_of_trades: 8,
        vwap: 1e7,
        ..NuamTrade::new("AAPL")
    };
    nuam_trade.set_sequence(5).unwrap();
    nuam_trade.set_tick_direction(Direction::Up);
    nuam_trade.set_extended_trading_hours(true);
    assert_eq!(
        nuam_trade.to_string(),
        format!(
            "NuamTrade{{{trade_base}, tradeStatTime=0, lastSignificantPrice=1.5, lastPriceForAll=2.25, \
             numberOfTrades=8, vwap=1.0E7}}"
        )
    );
}

#[test]
fn market_event_times_print_through_the_sdk() {
    let profile = Profile {
        halt_start_time: 1_700_000_000_123,
        halt_end_time: 1_700_000_100_000,
        ..Profile::new("AAPL")
    };
    let text = profile.to_string();
    for name in ["haltStartTime", "haltEndTime"] {
        assert!(is_time_without_millis(field(&text, name)), "{name} in {text}");
    }

    let mut greeks = Greeks::new("AAPL");
    greeks.set_time(1_700_000_000_789);
    greeks.set_sequence(5).unwrap();
    let text = greeks.to_string();
    assert!(is_time(field(&text, "time")), "{text}");
    assert!(field(&text, "time").ends_with(".789"), "{text}");
    assert_eq!((greeks.time(), greeks.sequence()), (1_700_000_000_789, 5));

    let nuam_trade = NuamTrade {
        trade_stat_time: 1_700_000_000_456,
        ..NuamTrade::new("AAPL")
    };
    let text = nuam_trade.to_string();
    assert!(field(&text, "tradeStatTime").ends_with(".456"), "{text}");
}

#[test]
fn packed_market_values_have_typed_accessors() {
    let mut profile = Profile::new("AAPL");
    profile.set_trading_status(TradingStatus::Halted);
    profile.set_short_sale_restriction(ShortSaleRestriction::Inactive);
    assert_eq!(
        (profile.trading_status(), profile.short_sale_restriction()),
        (TradingStatus::Halted, ShortSaleRestriction::Inactive)
    );
    assert!(profile.is_trading_halted() && !profile.is_short_sale_restricted());
    profile.set_short_sale_restriction(ShortSaleRestriction::Active);
    assert!(profile.is_short_sale_restricted());
    assert_eq!(profile.flags, (1 << 2) | 1);

    let mut summary = Summary::new("AAPL");
    summary.set_prev_day_close_price_type(PriceType::Indicative);
    assert_eq!(
        (summary.day_close_price_type(), summary.prev_day_close_price_type()),
        (PriceType::Regular, PriceType::Indicative)
    );

    let mut series = Series {
        index: 17,
        event_flags: EventFlags::SNAPSHOT_END,
        ..Series::new("AAPL")
    };
    series.set_time(1_700_000_000_000);
    series.set_sequence(3).unwrap();
    assert_eq!(
        (series.index, series.time(), series.sequence()),
        (17, 1_700_000_000_000, 3)
    );
    assert!(matches!(series.set_sequence(-1), Err(Error::InvalidArgument(_))));
    assert_eq!(IndexedEvent::index(&series), 17);
    assert_eq!(series.event_flags(), EventFlags::SNAPSHOT_END);

    let mut greeks = Greeks::new("AAPL");
    greeks.set_time(1_700_000_000_000);
    assert_eq!(TimeSeriesEvent::time(&greeks), 1_700_000_000_000);
    assert_eq!(MarketEvent::symbol(&greeks), "AAPL");
}

/// The strings Java 3.356 prints for `new TimeAndSale("AAPL")` and the other sale and candle events.
#[test]
fn new_sale_and_candle_events_print_as_in_java() {
    let sale = "eventTime=0, eventFlags=0x0, time=0, timeNanoPart=0, sequence=0, exchange=\\0, tradeId=0, price=NaN, \
                size=NaN, bid=NaN, ask=NaN, ESC='null', TTE=\\0, side=UNDEFINED, spread=false, ETH=false, \
                validTick=false, type=NEW";
    let candle = "eventTime=0, eventFlags=0x0, time=0, sequence=0, count=0, open=NaN, high=NaN, low=NaN, close=NaN, \
                  volume=NaN, vwap=NaN, bidVolume=NaN, askVolume=NaN, impVolatility=NaN, openInterest=NaN";
    let cases: [(String, String); 5] = [
        (
            TimeAndSale::new("AAPL").to_string(),
            format!("TimeAndSale{{AAPL, {sale}}}"),
        ),
        (
            NuamTimeAndSale::new("AAPL").to_string(),
            format!("NuamTimeAndSale{{AAPL, {sale}, matchId=0}}"),
        ),
        (
            OptionSale::new("AAPL").to_string(),
            "OptionSale{AAPL, eventTime=0, eventFlags=0x0, index=0x0, time=0, timeNanoPart=0, sequence=0, \
             exchange=\\0, price=NaN, size=NaN, bid=NaN, ask=NaN, ESC='null', TTE=\\0, side=UNDEFINED, spread=false, \
             ETH=false, validTick=false, type=NEW, underlyingPrice=NaN, volatility=NaN, delta=NaN, optionSymbol='null'}"
                .to_owned(),
        ),
        (
            Candle::new("AAPL{=d}").to_string(),
            format!("Candle{{AAPL{{=d}}, {candle}}}"),
        ),
        (
            DailyCandle::new("AAPL{=d}").to_string(),
            format!("DailyCandle{{AAPL{{=d}}, {candle}}}"),
        ),
    ];
    for (rust, java) in cases {
        assert_eq!(rust, java);
    }
}

/// Sets the sale values of Java's reference strings: the setters `setTradeThroughExempt('X')`,
/// `setAggressorSide(SELL)`, `setSpreadLeg(true)`, `setExtendedTradingHours(true)`, `setValidTick(true)` and
/// `setType(CORRECTION)`.
macro_rules! set_sale_flags {
    ($sale:expr) => {
        $sale.set_trade_through_exempt('X').unwrap();
        $sale.set_aggressor_side(Side::Sell);
        $sale.set_spread_leg(true);
        $sale.set_extended_trading_hours(true);
        $sale.set_valid_tick(true);
        $sale.set_time_and_sale_type(TimeAndSaleType::Correction);
        assert_eq!($sale.flags, 0x585d);
    };
}

/// The strings Java 3.356 prints for sale and candle events filled through the Java setters of the same names.
#[test]
fn filled_sale_and_candle_events_print_as_in_java() {
    let mut time_and_sale = TimeAndSale {
        time_nano_part: 7,
        exchange_code: 'Q',
        trade_id: 42,
        price: 1.5,
        size: 10.0,
        bid_price: 2.25,
        ask_price: 1e7,
        exchange_sale_conditions: Some("ESC".to_owned()),
        buyer: Some("buyer".to_owned()),
        seller: Some("seller".to_owned()),
        ..TimeAndSale::new("AAPL")
    };
    time_and_sale.set_sequence(5).unwrap();
    set_sale_flags!(time_and_sale);
    let sale = "AAPL, eventTime=0, eventFlags=0x0, time=0, timeNanoPart=7, sequence=5, exchange=Q, tradeId=42, \
                price=1.5, size=10.0, bid=2.25, ask=1.0E7, ESC='ESC', TTE=X, side=SELL, spread=true, ETH=true, \
                validTick=true, type=CORRECTION, buyer='buyer', seller='seller'";
    assert_eq!(time_and_sale.to_string(), format!("TimeAndSale{{{sale}}}"));

    let mut nuam = NuamTimeAndSale {
        time_nano_part: 7,
        exchange_code: 'Q',
        trade_id: 42,
        price: 1.5,
        size: 10.0,
        bid_price: 2.25,
        ask_price: 1e7,
        exchange_sale_conditions: Some("ESC".to_owned()),
        buyer: Some("buyer".to_owned()),
        seller: Some("seller".to_owned()),
        match_id: 99,
        ..NuamTimeAndSale::new("AAPL")
    };
    nuam.set_sequence(5).unwrap();
    set_sale_flags!(nuam);
    assert_eq!(nuam.to_string(), format!("NuamTimeAndSale{{{sale}, matchId=99}}"));

    // Java prints the buyer and the seller only when they are set, empty ones too.
    let empty = TimeAndSale {
        exchange_sale_conditions: Some(String::new()),
        buyer: Some(String::new()),
        seller: Some(String::new()),
        ..TimeAndSale::new("AAPL")
    };
    assert!(empty.to_string().ends_with(
        "ESC='', TTE=\\0, side=UNDEFINED, spread=false, ETH=false, \
                                         validTick=false, type=NEW, buyer='', seller=''}"
    ));
    let seller_only = TimeAndSale {
        seller: Some("s".to_owned()),
        ..TimeAndSale::new("AAPL")
    };
    assert!(
        seller_only.to_string().ends_with("type=NEW, seller='s'}"),
        "{seller_only}"
    );

    let mut option_sale = OptionSale {
        index: 17,
        time_nano_part: 7,
        exchange_code: 'Q',
        price: 1.5,
        size: 10.0,
        bid_price: 2.25,
        ask_price: 1e7,
        exchange_sale_conditions: Some("ESC".to_owned()),
        underlying_price: 100.0,
        volatility: 0.25,
        delta: 0.5,
        option_symbol: Some(".AAPL250117C150".to_owned()),
        ..OptionSale::new("AAPL")
    };
    option_sale.set_sequence(5).unwrap();
    set_sale_flags!(option_sale);
    assert_eq!(
        option_sale.to_string(),
        "OptionSale{AAPL, eventTime=0, eventFlags=0x0, index=0x11, time=0, timeNanoPart=7, sequence=5, exchange=Q, \
         price=1.5, size=10.0, bid=2.25, ask=1.0E7, ESC='ESC', TTE=X, side=SELL, spread=true, ETH=true, \
         validTick=true, type=CORRECTION, underlyingPrice=100.0, volatility=0.25, delta=0.5, \
         optionSymbol='.AAPL250117C150'}"
    );

    let candle = "AAPL{=d}, eventTime=0, eventFlags=0x0, time=0, sequence=5, count=3, open=1.5, high=1.0E7, low=1.0, \
                  close=10.0, volume=1000.0, vwap=1.75, bidVolume=400.0, askVolume=600.0, impVolatility=0.25, \
                  openInterest=77.0";
    let mut filled = Candle {
        count: 3,
        open: 1.5,
        high: 1e7,
        low: 1.0,
        close: 10.0,
        volume: 1000.0,
        vwap: 1.75,
        bid_volume: 400.0,
        ask_volume: 600.0,
        imp_volatility: 0.25,
        open_interest: 77.0,
        ..Candle::new("AAPL{=d}")
    };
    filled.set_sequence(5).unwrap();
    assert_eq!(filled.to_string(), format!("Candle{{{candle}}}"));
    let mut daily = DailyCandle {
        count: 3,
        open: 1.5,
        high: 1e7,
        low: 1.0,
        close: 10.0,
        volume: 1000.0,
        vwap: 1.75,
        bid_volume: 400.0,
        ask_volume: 600.0,
        imp_volatility: 0.25,
        open_interest: 77.0,
        ..DailyCandle::new("AAPL{=d}")
    };
    daily.set_sequence(5).unwrap();
    assert_eq!(daily.to_string(), format!("DailyCandle{{{candle}}}"));
}

#[test]
fn sale_values_have_typed_accessors() {
    let mut sale = TimeAndSale::new("AAPL");
    set_sale_flags!(sale);
    assert_eq!(
        (
            sale.trade_through_exempt(),
            sale.aggressor_side(),
            sale.time_and_sale_type()
        ),
        ('X', Side::Sell, TimeAndSaleType::Correction)
    );
    assert!(sale.is_spread_leg() && sale.is_extended_trading_hours() && sale.is_valid_tick());
    assert!(sale.is_correction() && !sale.is_new() && !sale.is_cancel());
    sale.set_time_and_sale_type(TimeAndSaleType::Cancel);
    sale.set_spread_leg(false);
    sale.set_trade_through_exempt('\u{ff}').unwrap();
    assert!(sale.is_cancel() && !sale.is_spread_leg() && sale.is_valid_tick());
    assert_eq!(
        (sale.trade_through_exempt(), sale.aggressor_side()),
        ('\u{ff}', Side::Sell)
    );
    assert!(matches!(
        sale.set_trade_through_exempt('\u{100}'),
        Err(Error::InvalidArgument(_))
    ));
    assert_eq!(sale.trade_through_exempt(), '\u{ff}');

    sale.set_time_nanos(1_700_000_000_123_456_789);
    assert_eq!(
        (sale.time(), sale.time_nano_part, sale.time_nanos()),
        (1_700_000_000_123, 456_789, 1_700_000_000_123_456_789)
    );
    assert_eq!(IndexedEvent::index(&sale), sale.index);
    assert_eq!(TimeSeriesEvent::time(&sale), 1_700_000_000_123);

    let mut option_sale = OptionSale {
        index: 17,
        ..OptionSale::new("AAPL")
    };
    option_sale.set_time(1_700_000_000_000);
    assert_eq!(
        (IndexedEvent::index(&option_sale), option_sale.time()),
        (17, 1_700_000_000_000)
    );
    assert_eq!(MarketEvent::symbol(&option_sale), "AAPL");

    let mut candle = Candle::new("AAPL{=d}");
    candle.set_time(1_700_000_000_000);
    assert_eq!(candle.index, 1_700_000_000 << 32);
    assert_eq!(TimeSeriesEvent::time(&candle), 1_700_000_000_000);

    let text = {
        let mut sale = TimeAndSale::new("AAPL");
        sale.set_time(1_700_000_000_789);
        sale.to_string()
    };
    assert!(
        is_time(field(&text, "time")) && field(&text, "time").ends_with(".789"),
        "{text}"
    );
}

/// The text that Java 3.356 prints for `OrderBase.baseFieldsToString` of a new order, after the source.
const NEW_ORDER_BASE: &str = "eventFlags=0x0, index=0x0, time=0, sequence=0, timeNanoPart=0, action=UNDEFINED, \
                              actionTime=0, orderId=0, auxOrderId=0, price=NaN, size=NaN, executedSize=NaN, count=0, \
                              exchange=\\0, side=UNDEFINED, scope=COMPOSITE, tradeId=0, tradePrice=NaN, tradeSize=NaN";

/// The strings Java 3.356 prints for `new Order("AAPL")` and the other order-book events.
#[test]
fn new_order_events_print_as_in_java() {
    let base = format!("AAPL, eventTime=0, source=DEFAULT, {NEW_ORDER_BASE}");
    let cases: [(String, String); 8] = [
        (OrderBase::new("AAPL").to_string(), format!("OrderBase{{{base}}}")),
        (
            Order::new("AAPL").to_string(),
            format!("Order{{{base}, marketMaker='null'}}"),
        ),
        (
            AnalyticOrder::new("AAPL").to_string(),
            format!(
                "AnalyticOrder{{{base}, marketMaker='null', icebergPeakSize=NaN, icebergHiddenSize=NaN, \
                 icebergExecutedSize=NaN, icebergType=UNDEFINED}}"
            ),
        ),
        (
            SpreadOrder::new("AAPL").to_string(),
            format!("SpreadOrder{{{base}, spreadSymbol='null'}}"),
        ),
        (
            OtcMarketsOrder::new("AAPL").to_string(),
            format!(
                "OtcMarketsOrder{{{base}, marketMaker='null', QAP=0, open=false, unsolicited=false, \
                 priceType=UNPRICED, saturated=false, autoEx=false, NMS=false}}"
            ),
        ),
        (
            NuamOrder::new("AAPL").to_string(),
            format!(
                "NuamOrder{{{base}, marketMaker='null', actorId=0, participantId=0, submitterId=0, \
                 onBehalfOfSubmitterId=0, clientOrderId='null', customerAccount='null', customerInfo='null', \
                 timeInForce=UNDEFINED, timeInForceData=0, triggerOrderBookId=0, triggerPrice=NaN, \
                 triggerSessionType=0, orderType=UNDEFINED, orderQuantity=NaN, displayQuantity=NaN, \
                 refreshQuantity=NaN, leavesQuantity=NaN, matchedQuantity=NaN}}"
            ),
        ),
        (
            MarketMaker::new("AAPL").to_string(),
            "MarketMaker{AAPL, eventTime=0, source=DEFAULT, eventFlags=0x0, index=0x0, time=0, exchange=\\0, \
             marketMaker='null', bidTime=0, bidPrice=NaN, bidSize=NaN, bidCount=0, askTime=0, askPrice=NaN, \
             askSize=NaN, askCount=0}"
                .to_owned(),
        ),
        (
            OrderImbalance::new("AAPL").to_string(),
            "OrderImbalance{AAPL, eventTime=0, source=DEFAULT, time=0, sequence=0, refPrice=NaN, pairedSize=NaN, \
             imbalanceSize=NaN, nearPrice=NaN, farPrice=NaN, imbalanceSide=UNDEFINED, auctionType=UNDEFINED}"
                .to_owned(),
        ),
    ];
    for (rust, java) in cases {
        assert_eq!(rust, java);
    }
}

/// The id of Java's order source `NTV`.
const NTV: i32 = 0x4e5456;

/// Fills an order as Java's reference strings do: `setIndex(17)`, `setSource(NTV)`, `setSequence(5)`,
/// `setTimeNanoPart(7)`, `setAction(NEW)`, `setOrderId(1001)`, `setAuxOrderId(1002)`, `setPrice(1.5)`, `setSize(10)`,
/// `setExecutedSize(2.25)`, `setCount(3)`, `setExchangeCode('Q')`, `setOrderSide(SELL)`, `setScope(ORDER)`,
/// `setTradeId(42)`, `setTradePrice(1.0E7)` and `setTradeSize(4.5)`.
macro_rules! fill_order_base {
    ($order:expr) => {
        $order.index = 17;
        $order.set_source(NTV).unwrap();
        $order.set_sequence(5).unwrap();
        $order.time_nano_part = 7;
        $order.set_action(OrderAction::New);
        $order.order_id = 1001;
        $order.aux_order_id = 1002;
        $order.price = 1.5;
        $order.size = 10.0;
        $order.executed_size = 2.25;
        $order.count = 3;
        $order.set_exchange_code('Q').unwrap();
        $order.set_order_side(Side::Sell);
        $order.set_scope(Scope::Order);
        $order.trade_id = 42;
        $order.trade_price = 1e7;
        $order.trade_size = 4.5;
        assert_eq!(($order.index, $order.flags), (0x4e_5456_0000_0011, 0xd1b));
    };
}

/// The strings Java 3.356 prints for order-book events filled through the Java setters of the same names.
#[test]
fn filled_order_events_print_as_in_java() {
    let base = "AAPL, eventTime=0, source=NTV, eventFlags=0x0, index=0x4e545600000011, time=0, sequence=5, \
                timeNanoPart=7, action=NEW, actionTime=0, orderId=1001, auxOrderId=1002, price=1.5, size=10.0, \
                executedSize=2.25, count=3, exchange=Q, side=SELL, scope=ORDER, tradeId=42, tradePrice=1.0E7, \
                tradeSize=4.5";

    let mut order_base = OrderBase::new("AAPL");
    fill_order_base!(order_base);
    assert_eq!(order_base.to_string(), format!("OrderBase{{{base}}}"));

    let mut order = Order {
        market_maker: Some("MM1".to_owned()),
        ..Order::new("AAPL")
    };
    fill_order_base!(order);
    assert_eq!(order.to_string(), format!("Order{{{base}, marketMaker='MM1'}}"));

    let mut analytic = AnalyticOrder {
        market_maker: Some("MM1".to_owned()),
        iceberg_peak_size: 1.5,
        iceberg_hidden_size: 2.25,
        iceberg_executed_size: 10.0,
        ..AnalyticOrder::new("AAPL")
    };
    fill_order_base!(analytic);
    analytic.set_iceberg_type(IcebergType::Synthetic);
    assert_eq!(
        analytic.to_string(),
        format!(
            "AnalyticOrder{{{base}, marketMaker='MM1', icebergPeakSize=1.5, icebergHiddenSize=2.25, \
             icebergExecutedSize=10.0, icebergType=SYNTHETIC}}"
        )
    );

    let mut spread = SpreadOrder {
        spread_symbol: Some("SPR".to_owned()),
        ..SpreadOrder::new("AAPL")
    };
    fill_order_base!(spread);
    assert_eq!(spread.to_string(), format!("SpreadOrder{{{base}, spreadSymbol='SPR'}}"));

    let mut otc = OtcMarketsOrder {
        market_maker: Some("MM1".to_owned()),
        quote_access_payment: 15,
        ..OtcMarketsOrder::new("AAPL")
    };
    fill_order_base!(otc);
    otc.set_open(true);
    otc.set_unsolicited(true);
    otc.set_otc_markets_price_type(OtcMarketsPriceType::Actual);
    otc.set_saturated(true);
    otc.set_auto_execution(true);
    otc.set_nms_conditional(true);
    assert_eq!(otc.otc_markets_flags, 0x77);
    assert_eq!(
        otc.to_string(),
        format!(
            "OtcMarketsOrder{{{base}, marketMaker='MM1', QAP=15, open=true, unsolicited=true, priceType=ACTUAL, \
             saturated=true, autoEx=true, NMS=true}}"
        )
    );

    let mut nuam = NuamOrder {
        market_maker: Some("MM1".to_owned()),
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
        trigger_price: 1.5,
        trigger_session_type: 7,
        order_quantity: 1e7,
        display_quantity: 2.25,
        refresh_quantity: 3.5,
        leaves_quantity: 4.5,
        matched_quantity: 5.5,
        ..NuamOrder::new("AAPL")
    };
    fill_order_base!(nuam);
    nuam.set_time_in_force(NuamTimeInForceType::Gtc);
    nuam.set_order_type(NuamOrderType::Limit);
    assert_eq!(nuam.nuam_flags, 0x21);
    assert_eq!(
        nuam.to_string(),
        format!(
            "NuamOrder{{{base}, marketMaker='MM1', actorId=1, participantId=2, submitterId=3, \
             onBehalfOfSubmitterId=4, clientOrderId='cid', customerAccount='acct', customerInfo='cinfo', \
             timeInForce=GTC, timeInForceData=5, triggerOrderBookId=6, triggerPrice=1.5, triggerSessionType=7, \
             orderType=LIMIT, orderQuantity=1.0E7, displayQuantity=2.25, refreshQuantity=3.5, leavesQuantity=4.5, \
             matchedQuantity=5.5}}"
        )
    );

    let mut market_maker = MarketMaker {
        bid_price: 1.5,
        bid_size: 10.0,
        bid_count: 3,
        ask_price: 2.25,
        ask_size: 1e7,
        ask_count: 4,
        ..MarketMaker::new("AAPL")
    };
    market_maker.set_exchange_code('Q').unwrap();
    market_maker.set_market_maker("MM1").unwrap();
    assert_eq!(
        market_maker.to_string(),
        "MarketMaker{AAPL, eventTime=0, source=DEFAULT, eventFlags=0x0, index=0x51004d4d31, time=0, exchange=Q, \
         marketMaker='MM1', bidTime=0, bidPrice=1.5, bidSize=10.0, bidCount=3, askTime=0, askPrice=2.25, \
         askSize=1.0E7, askCount=4}"
    );

    let mut imbalance = OrderImbalance {
        source_id: NTV,
        ref_price: 1.5,
        paired_size: 10.0,
        imbalance_size: 2.25,
        near_price: 1e7,
        far_price: 3.5,
        ..OrderImbalance::new("AAPL")
    };
    imbalance.set_sequence(5).unwrap();
    imbalance.set_imbalance_side(ImbalanceSide::Sell);
    imbalance.set_auction_type(AuctionType::Closing);
    assert_eq!(imbalance.flags, 18);
    assert_eq!(
        imbalance.to_string(),
        "OrderImbalance{AAPL, eventTime=0, source=NTV, time=0, sequence=5, refPrice=1.5, pairedSize=10.0, \
         imbalanceSize=2.25, nearPrice=1.0E7, farPrice=3.5, imbalanceSide=SELL, auctionType=CLOSING}"
    );
}

#[test]
fn order_values_have_typed_accessors() {
    let mut order = Order::new("AAPL");
    fill_order_base!(order);
    assert_eq!(
        (
            order.source(),
            order.action(),
            order.exchange_code(),
            order.order_side(),
            order.scope()
        ),
        (NTV, OrderAction::New, 'Q', Side::Sell, Scope::Order)
    );
    assert_eq!(
        (IndexedEvent::source(&order), IndexedEvent::index(&order)),
        (NTV, order.index)
    );
    assert_eq!(MarketEvent::symbol(&order), "AAPL");

    // A special source moves to bits 48 to 63 and back; the low bits stay.
    order.set_source(3).unwrap();
    assert_eq!((order.index, order.source()), (0x3_0000_0000_0011, 3));
    assert!(field(&order.to_string(), "source").starts_with("REGIONAL_BID"));
    order.set_source(0).unwrap();
    assert_eq!((order.index, order.source()), (17, 0));
    for bad in [10, 65535, -1, i32::MAX] {
        assert!(matches!(order.set_source(bad), Err(Error::InvalidArgument(_))), "{bad}");
    }
    assert_eq!(order.index, 17);

    // The flags keep 7 bits of the exchange code.
    order.set_exchange_code('\u{7f}').unwrap();
    assert!(matches!(
        order.set_exchange_code('\u{80}'),
        Err(Error::InvalidArgument(_))
    ));
    assert_eq!(
        (order.exchange_code(), order.order_side(), order.action()),
        ('\u{7f}', Side::Sell, OrderAction::New)
    );

    order.set_time_nanos(1_700_000_000_123_456_789);
    assert_eq!(
        (order.time(), order.time_nano_part, order.time_nanos(), order.sequence()),
        (1_700_000_000_123, 456_789, 1_700_000_000_123_456_789, 5)
    );

    // A source that Java refuses prints as its id.
    let imbalance = OrderImbalance {
        source_id: 10,
        ..OrderImbalance::new("AAPL")
    };
    assert_eq!(field(&imbalance.to_string(), "source"), "10");

    // The time of a market maker is the later of its bid and ask times.
    let mut market_maker = MarketMaker {
        bid_time: 1_700_000_000_000,
        ask_time: 1_700_000_001_000,
        ..MarketMaker::new("AAPL")
    };
    assert_eq!(
        (market_maker.time(), IndexedEvent::source(&market_maker)),
        (1_700_000_001_000, 0)
    );
    market_maker.set_market_maker("ABCD").unwrap();
    assert!(matches!(
        market_maker.set_market_maker("ABCDE"),
        Err(Error::InvalidArgument(_))
    ));
    assert!(matches!(
        market_maker.set_exchange_code('\u{10000}'),
        Err(Error::InvalidArgument(_))
    ));
    market_maker.set_exchange_code('\u{ffff}').unwrap();
    assert_eq!(
        (market_maker.exchange_code(), market_maker.market_maker().as_deref()),
        ('\u{ffff}', Some("ABCD"))
    );

    let analytic = AnalyticOrder {
        iceberg_flags: 1,
        ..AnalyticOrder::new("AAPL")
    };
    assert_eq!(analytic.iceberg_type(), IcebergType::Native);
}
