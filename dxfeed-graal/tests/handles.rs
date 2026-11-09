//! Handle ownership. One test only: the live-handle count is process-wide.

use std::cell::RefCell;

use dxfeed_graal::event::{Order, Quote, TimeAndSale};
use dxfeed_graal::model::{IndexedTxModel, TimeSeriesTxModel};
use dxfeed_graal::schedule::Schedule;
use dxfeed_graal::{DXEndpoint, DXFeedSubscription, Role, live_handles};

thread_local! {
    static KEPT: RefCell<Option<DXEndpoint>> = const { RefCell::new(None) };
}

#[test]
fn handles_are_released() {
    let before = live_handles();
    {
        let endpoint = DXEndpoint::create_with_role(Role::LocalHub).unwrap();
        let _feed = endpoint.get_feed();
        let _publisher = endpoint.get_publisher();
        assert_eq!(live_handles(), before + 3);
    }
    assert_eq!(live_handles(), before);

    // A feed taken from a chain keeps the endpoint and its handles; the last handle dropped releases them.
    {
        let feed = DXEndpoint::create_with_role(Role::LocalHub).unwrap().get_feed();
        assert_eq!(live_handles(), before + 3);
        let subscription = feed.create_subscription::<Quote>().unwrap();
        drop(feed);
        assert_eq!(live_handles(), before + 4);
        drop(subscription);
    }
    assert_eq!(live_handles(), before);

    // An endpoint dropped by a thread-local destructor, possibly after the crate's own thread guard has run.
    std::thread::spawn(|| {
        KEPT.with(|_| ()); // registered before the crate's guard, so destroyed after it
        let endpoint = DXEndpoint::create_with_role(Role::LocalHub).unwrap();
        KEPT.with(|kept| *kept.borrow_mut() = Some(endpoint));
    })
    .join()
    .unwrap();
    assert_eq!(live_handles(), before);

    // The sessions of a day are taken from an SDK list, which is freed apart from them.
    {
        let schedule = Schedule::get_instance_by_definition(
            "Test(name=Test;tz=America/New_York;td=12345;de=+0000;0=p04000930r09301600a16002000)",
        )
        .unwrap();
        let day = schedule.get_day_by_year_month_day(20261028).unwrap();
        let sessions = day.get_sessions().unwrap();
        assert_eq!(sessions.len(), 5);
        assert_eq!(live_handles(), before + 2 + 5);
        let _first = sessions[0].get_day().unwrap();
        assert_eq!(live_handles(), before + 2 + 5 + 1);
    }
    assert_eq!(live_handles(), before);

    // A standalone subscription owns one handle, attached or not; a detached one keeps no endpoint.
    {
        let subscription = DXFeedSubscription::<Quote>::new().unwrap();
        assert_eq!(live_handles(), before + 1);
        let feed = DXEndpoint::create_with_role(Role::LocalHub).unwrap().get_feed();
        subscription.attach(&feed).unwrap();
        subscription.detach(&feed).unwrap();
        drop(feed);
        assert_eq!(live_handles(), before + 1);
        drop(subscription);
    }
    assert_eq!(live_handles(), before);

    // A model keeps itself and its listener; the handles of its builders are released as it is built.
    {
        let endpoint = DXEndpoint::create_with_role(Role::LocalHub).unwrap();
        let model = IndexedTxModel::<Order>::new_builder()
            .with_feed(&endpoint.get_feed())
            .with_symbol("AAPL")
            .with_batch_processing(false)
            .with_listener(|_, _, _| {})
            .build()
            .unwrap();
        assert_eq!(live_handles(), before + 3 + 2);
        let series = TimeSeriesTxModel::<TimeAndSale>::new_builder()
            .with_feed(&endpoint.get_feed())
            .with_symbol("AAPL")
            .with_from_time(0)
            .with_listener(|_, _, _| {})
            .build()
            .unwrap();
        assert_eq!(live_handles(), before + 3 + 2 + 2);
        drop(endpoint);
        drop(model);
        drop(series);
    }
    assert_eq!(live_handles(), before);

    // The connectors are cloned from an SDK list, which is freed apart from them; a connector keeps no endpoint, and
    // the clones of one connector share its handle.
    {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let endpoint = DXEndpoint::create().unwrap();
        endpoint
            .connect(&format!("(127.0.0.1:{port})(127.0.0.1:{port})"))
            .unwrap();
        let connectors = endpoint.get_connectors().unwrap();
        assert_eq!(connectors.len(), 2);
        assert_eq!(live_handles(), before + 3 + 2);
        let _clone = connectors[0].clone();
        assert_eq!(live_handles(), before + 3 + 2);
        drop(endpoint);
        assert_eq!(live_handles(), before + 2);
        connectors[1].get_name().unwrap();
    }
    assert_eq!(live_handles(), before);

    // A connector listener owns one handle, released when it is removed or its endpoint is dropped, also when it was
    // added to a closed endpoint; a connector that a call gets is released when the closure drops it.
    #[cfg(feature = "experimental")]
    {
        let endpoint = DXEndpoint::create().unwrap();
        endpoint.close().unwrap();
        let id = endpoint.add_message_connector_listener(|_| {}).unwrap();
        assert_eq!(live_handles(), before + 3 + 1);
        endpoint.remove_message_connector_listener(id).unwrap();
        assert_eq!(live_handles(), before + 3);
        drop(endpoint);
        assert_eq!(live_handles(), before);

        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let endpoint = DXEndpoint::create().unwrap();
        let (sender, calls) = std::sync::mpsc::channel();
        endpoint
            .add_message_connector_listener(move |connector| {
                let _ = sender.send(connector.get_name().is_ok());
            })
            .unwrap();
        endpoint.connect(&format!("127.0.0.1:{port}")).unwrap();
        assert_eq!(calls.recv_timeout(std::time::Duration::from_secs(20)), Ok(true));
        drop(endpoint);
        assert_eq!(live_handles(), before);
    }

    // A model that cannot be built, without a symbol, releases its builders and its listener.
    {
        let result = IndexedTxModel::<Order>::new_builder()
            .with_listener(|_, _, _| {})
            .build();
        assert!(result.is_err());
    }
    assert_eq!(live_handles(), before);
    // The SDK still works after that.
    DXEndpoint::create_with_role(Role::LocalHub).unwrap();
}
