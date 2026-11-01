//! The model of the order book of one symbol (`OrderBookModel`) and its filter (`OrderBookModelFilter`).

use std::fmt;
use std::str::FromStr;
use std::sync::Mutex;

use dxfeed_graal_sys as sys;

use crate::error::{Error, Result, c_string};
use crate::event::Order;
use crate::executor::Executor;
use crate::feed::DXFeed;
use crate::handle::Handle;
use crate::isolate::{self, call, out_optional_string};
use crate::listener::{OrderBookListener, OrderBookListeners};
use crate::subscription::ListenerId;
use crate::subscription_controller::{attach_feed, detach_feed};

use super::ObservableListModel;

/// Which orders an [`OrderBookModel`] shows, by their [`Scope`](crate::event::Scope) (`OrderBookModelFilter`).
///
/// When a filter accepts several scopes, the orders of a more detailed scope hide those of a less detailed one, as
/// in Java: the individual orders hide the aggregate price levels of their exchange and market maker, which hide the
/// regional quote of their exchange, and any of these hides the composite quote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum OrderBookModelFilter {
    /// The composite quotes only (`COMPOSITE`).
    Composite,
    /// The regional quotes only (`REGIONAL`).
    Regional,
    /// The aggregate price levels only (`AGGREGATE`).
    Aggregate,
    /// The individual orders only (`ORDER`).
    Order,
    /// The composite and regional quotes (`COMPOSITE_REGIONAL`).
    CompositeRegional,
    /// The composite and regional quotes and the aggregate price levels (`COMPOSITE_REGIONAL_AGGREGATE`).
    CompositeRegionalAggregate,
    /// All the orders, the default (`ALL`).
    #[default]
    All,
}

impl OrderBookModelFilter {
    /// All the filters, in the order of Java.
    pub const ALL: [OrderBookModelFilter; 7] = [
        OrderBookModelFilter::Composite,
        OrderBookModelFilter::Regional,
        OrderBookModelFilter::Aggregate,
        OrderBookModelFilter::Order,
        OrderBookModelFilter::CompositeRegional,
        OrderBookModelFilter::CompositeRegionalAggregate,
        OrderBookModelFilter::All,
    ];

    /// The Java name of the filter, such as `COMPOSITE_REGIONAL` (`name`).
    #[doc(alias = "name")]
    pub fn get_name(self) -> &'static str {
        match self {
            OrderBookModelFilter::Composite => "COMPOSITE",
            OrderBookModelFilter::Regional => "REGIONAL",
            OrderBookModelFilter::Aggregate => "AGGREGATE",
            OrderBookModelFilter::Order => "ORDER",
            OrderBookModelFilter::CompositeRegional => "COMPOSITE_REGIONAL",
            OrderBookModelFilter::CompositeRegionalAggregate => "COMPOSITE_REGIONAL_AGGREGATE",
            OrderBookModelFilter::All => "ALL",
        }
    }

    fn to_sys(self) -> sys::dxfg_v4_order_book_model_filter_t {
        match self {
            OrderBookModelFilter::Composite => sys::COMPOSITE,
            OrderBookModelFilter::Regional => sys::REGIONAL,
            OrderBookModelFilter::Aggregate => sys::AGGREGATE,
            OrderBookModelFilter::Order => sys::ORDER,
            OrderBookModelFilter::CompositeRegional => sys::COMPOSITE_REGIONAL,
            OrderBookModelFilter::CompositeRegionalAggregate => sys::COMPOSITE_REGIONAL_AGGREGATE,
            OrderBookModelFilter::All => sys::ALL,
        }
    }

    fn from_sys(filter: sys::dxfg_v4_order_book_model_filter_t) -> Result<Self> {
        OrderBookModelFilter::ALL
            .into_iter()
            .find(|candidate| candidate.to_sys() == filter)
            .ok_or_else(|| Error::InvalidArgument(format!("unknown order book model filter {filter}")))
    }
}

impl fmt::Display for OrderBookModelFilter {
    /// The Java name, as [`get_name`](OrderBookModelFilter::get_name).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(self.get_name())
    }
}

impl FromStr for OrderBookModelFilter {
    type Err = Error;

    /// The filter of a Java name, such as `COMPOSITE_REGIONAL` (`valueOf`); [`Error::InvalidArgument`] for another
    /// text.
    fn from_str(name: &str) -> Result<Self> {
        OrderBookModelFilter::ALL
            .into_iter()
            .find(|filter| filter.get_name() == name)
            .ok_or_else(|| Error::InvalidArgument(format!("unknown order book model filter {name:?}")))
    }
}

/// A model of the order book of one symbol (`OrderBookModel`): it subscribes to the [`Order`] events of the symbol,
/// applies their snapshots and transactions, and keeps the buy and sell orders sorted by price, the top of each side
/// first.
///
/// Its lists only show a consistent book: the events of an incomplete snapshot or transaction wait until it ends, and
/// the event flags of the orders in the lists are zero. Its listeners are called after each change of the book: on
/// the executor of the model for the events, and on the calling thread for the changes made by
/// [`set_filter`](Self::set_filter), [`set_lot_size`](Self::set_lot_size), [`set_symbol`](Self::set_symbol) and
/// [`clear`](Self::clear).
///
/// As in Java, the model is not thread-safe apart from [`attach`](Self::attach), [`detach`](Self::detach) and
/// [`close`](Self::close): change it and read its lists in its listeners, or on the thread that runs a
/// [`ManualExecutor`](crate::ManualExecutor) of the model.
///
/// Dropping it closes it and releases its listeners, whose closures are not called afterwards; dropping it from
/// another thread therefore waits for a running call of a listener, so do not hold a lock its closure takes while
/// dropping it. It keeps the endpoints of the feeds it is attached to open.
///
/// ```no_run
/// use dxfeed_graal::DXFeed;
/// use dxfeed_graal::model::{OrderBookModel, OrderBookModelFilter};
///
/// # fn main() -> dxfeed_graal::Result<()> {
/// let model = OrderBookModel::new()?;
/// model.set_filter(OrderBookModelFilter::All)?;
/// model.set_symbol(Some("AAPL"))?;
/// let buy_orders = model.get_buy_orders()?;
/// model.add_listener(move || {
///     for order in buy_orders.to_vec().unwrap_or_default() {
///         println!("{order}");
///     }
/// })?;
/// model.attach(DXFeed::get_instance()?)?;
/// # Ok(())
/// # }
/// ```
pub struct OrderBookModel {
    handle: Handle<sys::dxfg_v4_order_book_model_t>,
    listeners: OrderBookListeners,
    /// The feeds the model is attached to; dropped last, as they may close their endpoints.
    feeds: Mutex<Vec<DXFeed>>,
}

impl OrderBookModel {
    /// A new model without a symbol, with the filter [`All`](OrderBookModelFilter::All) and the lot size 1
    /// (`OrderBookModel()`).
    #[doc(alias = "OrderBookModel")]
    pub fn new() -> Result<Self> {
        // SAFETY: the out-pointer is valid for writes.
        let handle = Handle::create(|t, out| unsafe { sys::dxfg_v4_OrderBookModel_new(t, out) })?;
        Ok(OrderBookModel {
            handle,
            listeners: OrderBookListeners::new("a listener of this order book model"),
            feeds: Mutex::new(Vec::new()),
        })
    }

    /// Attaches the model to `feed`, which then delivers the orders of its symbol (`attach`). The model keeps the
    /// feed until it is detached or closed.
    pub fn attach(&self, feed: &DXFeed) -> Result<()> {
        let m = self.handle.as_ptr();
        attach_feed(&self.feeds, feed, |f| {
            // SAFETY: the model and the feed are live.
            call(|t| unsafe { sys::dxfg_v4_OrderBookModel_attach(t, m, f) })?;
            Ok(true)
        })
    }

    /// Detaches the model from `feed` and lets go of it (`detach`).
    pub fn detach(&self, feed: &DXFeed) -> Result<()> {
        let m = self.handle.as_ptr();
        detach_feed(&self.feeds, feed, |f| {
            // SAFETY: the model and the feed are live.
            call(|t| unsafe { sys::dxfg_v4_OrderBookModel_detach(t, m, f) })
        })
    }

    /// Detaches the model from all feeds for good, lets go of them and removes its listeners; closing it again does
    /// nothing (`close`). Dropping the model closes it too.
    pub fn close(&self) -> Result<()> {
        let mut feeds = crate::lock(&self.feeds);
        self.close_model()?;
        feeds.clear();
        drop(feeds);
        self.remove_listeners();
        Ok(())
    }

    fn close_model(&self) -> Result<()> {
        let m = self.handle.as_ptr();
        // SAFETY: the model is live.
        call(|t| unsafe { sys::dxfg_v4_OrderBookModel_close(t, m) })
    }

    fn remove_listeners(&self) {
        let m = self.handle.as_ptr();
        self.listeners.remove_all("OrderBookModel", |l| {
            // SAFETY: the model and the listener are live.
            call(|t| unsafe { sys::dxfg_v4_OrderBookModel_removeListener(t, m, l) })
        });
    }

    /// The executor that calls the listeners; `None` if it is the executor of the feed (`getExecutor`).
    #[doc(alias = "getExecutor")]
    pub fn get_executor(&self) -> Result<Option<Executor>> {
        let m = self.handle.as_ptr();
        // SAFETY: the model is live and the out-pointer is valid for writes.
        let handle = Handle::create_optional(|t, out| unsafe { sys::dxfg_v4_OrderBookModel_getExecutor(t, m, out) })?;
        Ok(handle.map(Executor::from_handle))
    }

    /// Calls the listeners on `executor` from now on, such as a thread pool or a
    /// [`ManualExecutor`](crate::ManualExecutor), or on the executor of the feed for `None` (`setExecutor`). The model
    /// keeps the executor.
    #[doc(alias = "setExecutor")]
    pub fn set_executor(&self, executor: Option<&Executor>) -> Result<()> {
        let m = self.handle.as_ptr();
        let e = executor.map_or(std::ptr::null_mut(), |executor| executor.handle.as_ptr());
        // SAFETY: the model and the executor are live.
        call(|t| unsafe { sys::dxfg_v4_OrderBookModel_setExecutor(t, m, e) })
    }

    /// Removes all the orders and unsubscribes the model, as [`set_symbol`](Self::set_symbol)`(None)` (`clear`).
    pub fn clear(&self) -> Result<()> {
        let m = self.handle.as_ptr();
        // SAFETY: the model is live.
        call(|t| unsafe { sys::dxfg_v4_OrderBookModel_clear(t, m) })
    }

    /// Which orders the model shows (`getFilter`).
    #[doc(alias = "getFilter")]
    pub fn get_filter(&self) -> Result<OrderBookModelFilter> {
        let mut filter = sys::ALL;
        let m = self.handle.as_ptr();
        // SAFETY: the model is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_OrderBookModel_getFilter(t, m, &mut filter) })?;
        OrderBookModelFilter::from_sys(filter)
    }

    /// Shows only the orders that `filter` accepts (`setFilter`).
    #[doc(alias = "setFilter")]
    pub fn set_filter(&self, filter: OrderBookModelFilter) -> Result<()> {
        let m = self.handle.as_ptr();
        // SAFETY: the model is live.
        call(|t| unsafe { sys::dxfg_v4_OrderBookModel_setFilter(t, m, filter.to_sys()) })
    }

    /// The symbol of the book, or `None` if the model is not subscribed (`getSymbol`).
    #[doc(alias = "getSymbol")]
    pub fn get_symbol(&self) -> Result<Option<String>> {
        let m = self.handle.as_ptr();
        // SAFETY: the model is live and the out-pointer is valid for writes.
        out_optional_string("OrderBookModel", |t, out| unsafe {
            sys::dxfg_v4_OrderBookModel_getSymbol(t, m, out)
        })
    }

    /// Subscribes the model to the orders of `symbol`, or unsubscribes it with `None`, which removes its orders
    /// (`setSymbol`).
    ///
    /// Returns [`Error::InvalidArgument`] for a NUL byte.
    #[doc(alias = "setSymbol")]
    pub fn set_symbol(&self, symbol: Option<&str>) -> Result<()> {
        let symbol = symbol.map(|symbol| c_string(symbol, "the symbol")).transpose()?;
        let s = symbol.as_ref().map_or(std::ptr::null(), |symbol| symbol.as_ptr());
        let m = self.handle.as_ptr();
        // SAFETY: the model is live and the symbol is NULL or NUL-terminated.
        call(|t| unsafe { sys::dxfg_v4_OrderBookModel_setSymbol(t, m, s) })
    }

    /// The lot size, 1 by default: the multiplier of the sizes of composite, regional and aggregate orders
    /// (`getLotSize`).
    #[doc(alias = "getLotSize")]
    pub fn get_lot_size(&self) -> Result<i32> {
        let mut lot_size = 0;
        let m = self.handle.as_ptr();
        // SAFETY: the model is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_OrderBookModel_getLotSize(t, m, &mut lot_size) })?;
        Ok(lot_size)
    }

    /// Sets the lot size (`setLotSize`).
    ///
    /// Returns [`Error::Java`] for a lot size less than 1.
    #[doc(alias = "setLotSize")]
    pub fn set_lot_size(&self, lot_size: i32) -> Result<()> {
        let m = self.handle.as_ptr();
        // SAFETY: the model is live.
        call(|t| unsafe { sys::dxfg_v4_OrderBookModel_setLotSize(t, m, lot_size) })
    }

    /// The live list of the buy orders, the bid side, from the highest price (`getBuyOrders`).
    #[doc(alias = "getBuyOrders")]
    pub fn get_buy_orders(&self) -> Result<ObservableListModel<Order>> {
        let m = self.handle.as_ptr();
        // SAFETY: the model is live and the out-pointer is valid for writes.
        let handle = Handle::create(|t, out| unsafe { sys::dxfg_v4_OrderBookModel_getBuyOrders(t, m, out) })?;
        Ok(ObservableListModel::new(handle))
    }

    /// The live list of the sell orders, the offer side, from the lowest price (`getSellOrders`).
    #[doc(alias = "getSellOrders")]
    pub fn get_sell_orders(&self) -> Result<ObservableListModel<Order>> {
        let m = self.handle.as_ptr();
        // SAFETY: the model is live and the out-pointer is valid for writes.
        let handle = Handle::create(|t, out| unsafe { sys::dxfg_v4_OrderBookModel_getSellOrders(t, m, out) })?;
        Ok(ObservableListModel::new(handle))
    }

    /// Adds a listener of the changes of the book and returns its id (`addListener`); a closed model does not add it.
    ///
    /// The listener is called after each change, on the executor of the model or on the thread that changes the
    /// model; its calls do not overlap, and a change it makes itself calls it once more after it returns. Read the
    /// book in it with [`get_buy_orders`](Self::get_buy_orders) and [`get_sell_orders`](Self::get_sell_orders) or
    /// lists taken before. A panic in `listener` is caught and goes to [`ErrorManager`](crate::ErrorManager).
    #[doc(alias = "addListener")]
    pub fn add_listener(&self, listener: impl FnMut() + Send + 'static) -> Result<ListenerId> {
        let m = self.handle.as_ptr();
        self.listeners.add(OrderBookListener::new(Box::new(listener))?, |l| {
            // SAFETY: the model and the listener are live.
            call(|t| unsafe { sys::dxfg_v4_OrderBookModel_addListener(t, m, l) })
        })
    }

    /// Removes a listener, waiting for its running call unless called from it (`removeListener`).
    ///
    /// Returns [`Error::InvalidArgument`] if the listener is not in this model.
    #[doc(alias = "removeListener")]
    pub fn remove_listener(&self, id: ListenerId) -> Result<()> {
        let m = self.handle.as_ptr();
        self.listeners.remove(id, |l| {
            // SAFETY: the model and the listener are live.
            call(|t| unsafe { sys::dxfg_v4_OrderBookModel_removeListener(t, m, l) })
        })
    }
}

impl Drop for OrderBookModel {
    fn drop(&mut self) {
        if isolate::exiting() {
            // No SDK calls while the process exits; the listeners leak, as the SDK may still call their trampoline.
            return;
        }
        crate::error_manager::report_release("OrderBookModel", self.close_model());
        self.remove_listeners();
    }
}

impl fmt::Debug for OrderBookModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OrderBookModel").finish_non_exhaustive()
    }
}
