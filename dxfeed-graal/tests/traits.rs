//! The event types implement the traits that their Java classes implement: a lost implementation fails to compile.

use dxfeed_graal::event::{
    AnalyticOrder, Candle, Configuration, DailyCandle, Greeks, IndexedEvent, LastingEvent, MarketEvent, MarketMaker,
    NuamOrder, NuamTimeAndSale, NuamTrade, OptionSale, Order, OrderBase, OrderImbalance, OtcMarketsOrder, Profile,
    Quote, Series, SpreadOrder, Summary, TextConfiguration, TheoPrice, TimeAndSale, TimeSeriesEvent, Trade, TradeETH,
    Underlying,
};

fn market<E: MarketEvent>() {}
fn lasting<E: LastingEvent>() {}
fn indexed<E: IndexedEvent>() {}
fn time_series<E: TimeSeriesEvent>() {}

#[test]
fn market_events() {
    market::<Quote>();
    market::<Profile>();
    market::<Summary>();
    market::<Greeks>();
    market::<Underlying>();
    market::<TheoPrice>();
    market::<Trade>();
    market::<TradeETH>();
    market::<TimeAndSale>();
    market::<OrderBase>();
    market::<Order>();
    market::<AnalyticOrder>();
    market::<OtcMarketsOrder>();
    market::<SpreadOrder>();
    market::<Series>();
    market::<OptionSale>();
    market::<MarketMaker>();
    market::<NuamOrder>();
    market::<NuamTimeAndSale>();
    market::<NuamTrade>();
    market::<OrderImbalance>();
}

#[test]
fn lasting_events() {
    lasting::<Quote>();
    lasting::<Profile>();
    lasting::<Summary>();
    lasting::<Greeks>();
    lasting::<Candle>();
    lasting::<DailyCandle>();
    lasting::<Underlying>();
    lasting::<TheoPrice>();
    lasting::<Trade>();
    lasting::<TradeETH>();
    lasting::<Configuration>();
    lasting::<TextConfiguration>();
    lasting::<NuamTrade>();
}

#[test]
fn indexed_events() {
    indexed::<Greeks>();
    indexed::<Candle>();
    indexed::<DailyCandle>();
    indexed::<Underlying>();
    indexed::<TheoPrice>();
    indexed::<TimeAndSale>();
    indexed::<OrderBase>();
    indexed::<Order>();
    indexed::<AnalyticOrder>();
    indexed::<OtcMarketsOrder>();
    indexed::<SpreadOrder>();
    indexed::<Series>();
    indexed::<OptionSale>();
    indexed::<MarketMaker>();
    indexed::<NuamOrder>();
    indexed::<NuamTimeAndSale>();
}

#[test]
fn time_series_events() {
    time_series::<Greeks>();
    time_series::<Candle>();
    time_series::<DailyCandle>();
    time_series::<Underlying>();
    time_series::<TheoPrice>();
    time_series::<TimeAndSale>();
    time_series::<NuamTimeAndSale>();
}
