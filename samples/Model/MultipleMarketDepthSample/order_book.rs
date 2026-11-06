use dxfeed_graal::event::{EventKind, OrderEvent};

/// The buy and sell orders of a book, best first.
pub struct OrderBook<O: OrderEvent + EventKind + Sync> {
    pub buy: Vec<O>,
    pub sell: Vec<O>,
}

impl<O: OrderEvent + EventKind + Sync> OrderBook<O> {
    pub fn new(buy_orders: &[O], sell_orders: &[O]) -> Self {
        OrderBook {
            buy: buy_orders.to_vec(),
            sell: sell_orders.to_vec(),
        }
    }
}
