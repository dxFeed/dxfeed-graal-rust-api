//! The public types that cross threads stay `Send` and `Sync`: losing either breaks the code of the users.

use dxfeed_graal::event::{Candle, Order, Quote, TimeAndSale};
use dxfeed_graal::history::{HistoryEndpoint, HistoryEndpointBuilder};
use dxfeed_graal::ipf::live::{InstrumentProfileCollector, InstrumentProfileConnection};
use dxfeed_graal::ipf::{InstrumentProfile, InstrumentProfileReader, InstrumentProfileWriter};
use dxfeed_graal::schedule::{Day, Schedule, Session};
use dxfeed_graal::{
    AuthToken, DXEndpoint, DXEndpointBuilder, DXFeed, DXFeedSubscription, DXFeedTimeSeriesSubscription, DXPublisher,
    Error, ErrorInfo, Event, EventStream, Executor, JavaException, ManualExecutor, MessageConnector,
    ObservableSubscription, Promise, StreamOptions, Symbol, TaggedEventStream, TimeFormat, TimePeriod, TimeZone,
};

fn send_sync<T: Send + Sync>() {}

fn send<T: Send>() {}

#[test]
fn the_handles_are_send_and_sync() {
    send_sync::<DXEndpoint>();
    send_sync::<DXEndpointBuilder>();
    send_sync::<DXFeed>();
    send_sync::<DXPublisher>();
    send_sync::<DXFeedSubscription<Quote>>();
    send_sync::<DXFeedSubscription<Event>>();
    send_sync::<DXFeedTimeSeriesSubscription<Candle>>();
    send_sync::<ObservableSubscription>();
    send_sync::<Promise<Vec<TimeAndSale>>>();
    send_sync::<Executor>();
    send_sync::<ManualExecutor>();
    send_sync::<MessageConnector>();
    send_sync::<AuthToken>();
    send_sync::<TimeFormat>();
    send_sync::<TimePeriod>();
    send_sync::<TimeZone>();
    send_sync::<Schedule>();
    send_sync::<Day>();
    send_sync::<Session>();
    send_sync::<HistoryEndpoint>();
    send_sync::<HistoryEndpointBuilder>();
    send_sync::<InstrumentProfileCollector>();
    send_sync::<InstrumentProfileConnection>();
    send_sync::<InstrumentProfileReader>();
    send_sync::<InstrumentProfileWriter>();
}

#[test]
fn the_values_are_send_and_sync() {
    send_sync::<Error>();
    send_sync::<JavaException>();
    send_sync::<ErrorInfo>();
    send_sync::<Event>();
    send_sync::<Quote>();
    send_sync::<Order>();
    send_sync::<Symbol>();
    send_sync::<InstrumentProfile>();
    send_sync::<StreamOptions>();
}

#[test]
fn the_streams_are_send() {
    send::<EventStream<Quote>>();
    send::<TaggedEventStream>();
}

#[cfg(feature = "experimental")]
#[test]
fn the_models_are_send_and_sync() {
    use dxfeed_graal::model::{IndexedTxModel, MarketDepthModel, OrderBookModel, TimeSeriesTxModel};
    send_sync::<IndexedTxModel<Order>>();
    send_sync::<TimeSeriesTxModel<Candle>>();
    send_sync::<MarketDepthModel<Order>>();
    send_sync::<OrderBookModel>();
}
