//! Replays the quotes of a symbol during the Flash Crash, from 2010-05-06 14:47:48 to 14:48:02 EST, with the
//! on-demand service, like `OnDemandSample` of the other dxFeed APIs. The service runs on the default on-demand
//! endpoint, configured by the `dxfeed.address`, `dxfeed.user` and `dxfeed.password` system properties.
//!
//! Usage: `cargo run --example on_demand -- [SYMBOL] [ADDRESS]`, by default `ACN`. ADDRESS needs an
//! `(ondemand:<address>)` component, else the replay fails as not connected. Without the ADDRESS argument the address
//! is `dxfeed.address` of the `dxfeed.system.properties` file (in the current directory or named by the
//! `DXFEED_dxfeed.system.properties` environment variable) or of the `DXFEED_dxfeed.address` environment variable if
//! it has an `(ondemand:<address>)` component, else `(ondemand:demo.dxfeed.com:7680)`. The user and the password are
//! `dxfeed.user` and `dxfeed.password`, by default `demo`. The program stops with an error if the replay does not
//! move for 30 seconds.

use std::process::ExitCode;
use std::thread;
use std::time::Duration;

use dxfeed_graal::event::Quote;
use dxfeed_graal::ondemand::OnDemandService;
use dxfeed_graal::{TimeFormat, system};

/// How many seconds the replay may stand still before the program gives up.
const STALL_LIMIT: u32 = 30;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let symbol = args.next().unwrap_or_else(|| "ACN".into());
    match run(&symbol, args.next()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(symbol: &str, address: Option<String>) -> dxfeed_graal::Result<()> {
    let address = address_or_default(address)?;
    // The on-demand endpoint is configured by the system properties when it is first used.
    system::set_property("dxfeed.address", &address)?;
    for property in ["dxfeed.user", "dxfeed.password"] {
        if system::get_property(property)?.is_none() {
            system::set_property(property, "demo")?;
        }
    }
    // The quotes carry the time of the replayed events.
    system::set_property("dxendpoint.eventTime", "true")?;

    let on_demand = OnDemandService::get_instance()?;
    let subscription = on_demand.get_endpoint().get_feed().create_subscription::<Quote>()?;
    let time_format = TimeFormat::DEFAULT.with_millis().with_time_zone();
    subscription.add_event_listener(move |quotes| {
        for quote in quotes {
            let time = time_format.format(quote.event_time()).unwrap_or_default();
            println!(
                "{time} : {} bid {} / ask {}",
                quote.symbol(),
                quote.bid_price(),
                quote.ask_price()
            );
        }
    })?;
    subscription.add_symbols([symbol])?;

    let from = time_format.parse("2010-05-06 14:47:48.000 EST")?;
    let to = time_format.parse("2010-05-06 14:48:02.000 EST")?;
    on_demand.replay(from)?;
    let (mut last, mut stalled) = (on_demand.get_time()?, 0);
    loop {
        let time = on_demand.get_time()?;
        if time >= to {
            break;
        }
        stalled = if time == last { stalled + 1 } else { 0 };
        last = time;
        if stalled == STALL_LIMIT {
            eprintln!("The replay did not move for {STALL_LIMIT} seconds.");
            on_demand.get_endpoint().close_and_await_termination()?;
            return Err(dxfeed_graal::Error::Timeout);
        }
        println!(
            "Current state is {}, on-demand time is {}",
            on_demand.get_endpoint().get_state()?,
            time_format.format(time)?
        );
        thread::sleep(Duration::from_secs(1));
    }
    on_demand.get_endpoint().close_and_await_termination()
}

/// The ADDRESS argument; without it, `dxfeed.address` of the `dxfeed.system.properties` file or of the
/// `DXFEED_dxfeed.address` environment variable if it names an on-demand provider, and the demo provider as the last
/// resort.
fn address_or_default(address: Option<String>) -> dxfeed_graal::Result<String> {
    if let Some(address) = address {
        return Ok(address);
    }
    Ok(system::get_property("dxfeed.address")?
        .filter(|address| address.contains("ondemand:"))
        .unwrap_or_else(|| "(ondemand:demo.dxfeed.com:7680)".into()))
}
