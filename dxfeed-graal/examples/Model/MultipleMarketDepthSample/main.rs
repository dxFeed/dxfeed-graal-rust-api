//! This sample program demonstrates how to use the `MultipleMarketDepthModel`.
//!
//! Usage: `cargo run --example MultipleMarketDepthSample`

mod multiple_market_depth_model;
mod order_book;

use std::time::Duration;

use dxfeed_graal::event::{IndexedEventSource, Order, OrderSource};
use dxfeed_graal::{DXEndpoint, DXFeed, Symbol};

use multiple_market_depth_model::MultipleMarketDepthModel;
use order_book::OrderBook;

fn main() -> dxfeed_graal::Result<()> {
    let address = "demo.dxfeed.com:7300";

    let symbol = Symbol::indexed("AAPL", OrderSource::NTV);
    let print_book = |book_symbol: &Symbol, book: &OrderBook<Order>| {
        let mut result = String::new();

        result += &format!("{book_symbol}:\n");

        let max_count = book.buy.len().max(book.sell.len());

        for i in 0..max_count {
            let buy_table = if i < book.buy.len() {
                format!(
                    "Buy  [Source: {}, Size: {:8.2}, Price: {:8.4}]",
                    get_source(&book.buy[i]),
                    book.buy[i].size,
                    book.buy[i].price
                )
            } else {
                "Buy  [None]".to_string()
            };
            let sell_table = if i < book.sell.len() {
                format!(
                    "Sell  [Source: {}, Size: {:8.2}, Price: {:8.4}]",
                    get_source(&book.sell[i]),
                    book.sell[i].size,
                    book.sell[i].price
                )
            } else {
                "Sell  [None]".to_string()
            };

            result += &format!("{buy_table} \t {sell_table}\n");
        }

        println!("{result}");
    };

    let model = MultipleMarketDepthModel::<Order>::new_builder()
        .with_feed(DXFeed::get_instance()?)
        .with_depth_limit(10)
        .with_aggregation_period(Duration::from_secs(10))
        .with_listener(print_book)
        .build();
    model.add_symbol(symbol.clone())?;
    model.add_symbol(Symbol::indexed("TSLA", OrderSource::NTV))?;

    DXEndpoint::get_instance()?.connect(address)?;

    println!("Press [ENTER] to print {symbol} order book manually.");

    loop {
        // Stdin of Rust reports the end of input, which stops the loop; the C++ sample waits on it forever.
        if !matches!(std::io::stdin().read_line(&mut String::new()), Ok(n) if n > 0) {
            break;
        }

        if let Some(book) = model.try_get_book(&symbol) {
            // The lock keeps the lines of the manual print together with the books that the listener prints.
            let _stdout = std::io::stdout().lock();
            println!("=============================Print manually=============================");
            print_book(&symbol, &book);
            println!("========================================================================");
        }
    }

    Ok(())
}

/// The source of an order.
fn get_source(order: &Order) -> IndexedEventSource {
    order.source()
}
