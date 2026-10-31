//! The event types implement the traits that their Java classes implement: a lost implementation fails to compile.

use dxfeed_graal::Event;
use dxfeed_graal::event::{
    AnalyticOrder, BaseEvent, Candle, CandleRef, Configuration, DailyCandle, Greeks, IndexedEvent, LastingEvent,
    MarketEvent, MarketMaker, MessageRef, NuamOrder, NuamTimeAndSale, NuamTrade, OptionSale, Order, OrderBase,
    OrderBaseRef, OrderEvent, OrderImbalance, OrderRef, OtcMarketsOrder, Profile, Quote, QuoteRef, Series, SpreadOrder,
    SpreadOrderRef, Summary, TextConfiguration, TextConfigurationRef, TheoPrice, TimeAndSale, TimeSeriesEvent, Trade,
    TradeETH, Underlying,
};

fn market<E: MarketEvent>() {}
fn lasting<E: LastingEvent>() {}
fn indexed<E: IndexedEvent>() {}
fn time_series<E: TimeSeriesEvent>() {}
fn base<E: BaseEvent>() {}
fn order_event<E: OrderEvent>() {}

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

#[test]
fn views_implement_the_traits_of_their_types() {
    market::<OrderBaseRef>();
    indexed::<OrderBaseRef>();
    order_event::<OrderBaseRef>();
    market::<QuoteRef>();
    lasting::<QuoteRef>();
    lasting::<TextConfigurationRef>();
    indexed::<OrderRef>();
    time_series::<CandleRef>();
    order_event::<OrderRef>();
    order_event::<SpreadOrderRef>();
    base::<MessageRef>();
}

#[test]
fn every_event_is_a_base_event() {
    base::<Candle>();
    base::<DailyCandle>();
    base::<Configuration>();
    base::<TextConfiguration>();
    base::<Event>();
}
