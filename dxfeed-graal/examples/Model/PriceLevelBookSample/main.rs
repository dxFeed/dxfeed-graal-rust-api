//! Usage: `cargo run --example PriceLevelBookSample`

mod price_level;
mod price_level_book;

use std::time::Duration;

use dxfeed_graal::event::OrderSource;
use dxfeed_graal::{DXEndpoint, DXFeed};

use price_level::PriceLevel;
use price_level_book::PriceLevelBook;

fn main() -> dxfeed_graal::Result<()> {
    let address = "demo.dxfeed.com:7300";

    let symbol = "MSFT";
    let source = OrderSource::NTV;

    let _book = PriceLevelBook::new_builder()
        .with_feed(DXFeed::get_instance()?)
        .with_symbol(symbol)
        .with_source(source)
        .with_depth_limit(10)
        .with_aggregation_period(Duration::from_secs(10))
        .with_listener(|buy_levels: &[PriceLevel], sell_levels: &[PriceLevel]| {
            if buy_levels.is_empty() && sell_levels.is_empty() {
                return;
            }

            println!("{:=^66}", "");
            println!("{:^31} || {:^31}", "ASK", "BID");
            println!("{0:^15}|{1:^15} || {0:^15}|{1:^15}", "Price", "Size");
            println!("{:-^66}", "");

            let mut buy_it = buy_levels.iter().peekable();
            let mut sell_it = sell_levels.iter().peekable();

            while buy_it.peek().is_some() || sell_it.peek().is_some() {
                let mut row = String::new();

                if let Some(level) = buy_it.next() {
                    row += &format!("{:>14.4} | {:<14.2}", level.get_price(), level.get_size());
                } else {
                    row += &format!("{:>14} | {:<14}", "", "");
                }

                row += " || ";

                if let Some(level) = sell_it.next() {
                    row += &format!("{:>14.4} | {:<14.2}", level.get_price(), level.get_size());
                } else {
                    row += &format!("{:>14} | {:<14}", "", "");
                }

                println!("{row}");
            }

            println!("{:=^66}", "");
        })
        .build()?;

    DXEndpoint::get_instance()?.connect(address)?;

    let _ = std::io::stdin().read_line(&mut String::new());
    Ok(())
}
