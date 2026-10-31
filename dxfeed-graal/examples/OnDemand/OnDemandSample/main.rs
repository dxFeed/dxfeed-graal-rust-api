//! A sample that uses dxFeed on-demand history data replay service API shows Accenture symbol "ACN" drops
//! under $1 on May 6, 2010 "Flashcrash" from 14:47:48 to 14:48:02 EST.
//! The properties file is read from the current directory, so run the sample from its directory:
//! `cargo run --example OnDemandSample`.

use std::time::Duration;

use dxfeed_graal::TimeFormat;
use dxfeed_graal::event::Quote;
use dxfeed_graal::ondemand::OnDemandService;

fn main() -> dxfeed_graal::Result<()> {
    // get on-demand-only data feed
    let on_demand = OnDemandService::get_instance()?;
    let feed = on_demand.get_endpoint().get_feed();

    // subscribe to Accenture symbol ACN to print its quotes
    let sub = feed.create_subscription::<Quote>()?;

    sub.add_event_listener(|events| {
        for quote in events {
            println!(
                "{} : {} bid {:?} / ask {:?}",
                TimeFormat::DEFAULT
                    .with_millis()
                    .with_time_zone()
                    .format(quote.event_time())
                    .unwrap(),
                quote.symbol(),
                quote.bid_price(),
                quote.ask_price()
            );
        }
    })?;

    sub.add_symbols(["ACN"])?;

    // Watch Accenture drop under $1 on May 6, 2010 "Flashcrash" from 14:47:48 to 14:48:02 EST
    let from = TimeFormat::DEFAULT
        .with_millis()
        .with_time_zone()
        .parse("2010-05-06 14:47:48.000 EST")?;
    let to = TimeFormat::DEFAULT
        .with_millis()
        .with_time_zone()
        .parse("2010-05-06 14:48:02.000 EST")?;

    // switch into historical on-demand data replay mode
    on_demand.replay(from)?;

    // replaying events until end time reached
    while on_demand.get_time()? < to {
        println!(
            "Current state is {}, on-demand time is {}",
            on_demand.get_endpoint().get_state()?,
            TimeFormat::DEFAULT
                .with_millis()
                .with_time_zone()
                .format(on_demand.get_time()?)?
        );

        std::thread::sleep(Duration::from_secs(1));
    }

    // close endpoint completely to release resources and shutdown GraalVM
    on_demand.get_endpoint().close_and_await_termination()?;

    Ok(())
}
