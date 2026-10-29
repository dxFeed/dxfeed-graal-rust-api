//! Endpoints with the `LOCAL_HUB` role: no network needed.

use std::sync::atomic::{AtomicUsize, Ordering};

use dxfeed_graal::{Endpoint, ManualExecutor, Role};

/// Builds a `LOCAL_HUB` endpoint with a name unique in this process and its own manual executor.
fn local_hub() -> (Endpoint, ManualExecutor) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let executor = ManualExecutor::new().unwrap();
    let endpoint = Endpoint::builder()
        .with_role(Role::LocalHub)
        .with_name(&format!("local-hub-{}", NEXT.fetch_add(1, Ordering::Relaxed)))
        .with_executor(&executor)
        .build()
        .unwrap();
    (endpoint, executor)
}

#[test]
fn local_hub_builds() {
    let (endpoint, executor) = local_hub();
    assert_eq!(endpoint.role().unwrap(), Role::LocalHub);
    endpoint.feed().unwrap();
    endpoint.publisher().unwrap();
    executor.process_all_pending().unwrap();
}

#[test]
fn create_with_role() {
    assert_eq!(
        Endpoint::create(Role::Publisher).unwrap().role().unwrap(),
        Role::Publisher
    );
}

#[test]
fn close_then_drop() {
    let (endpoint, _executor) = local_hub();
    endpoint.close().unwrap();
    endpoint.close().unwrap();
}

#[test]
fn endpoint_moves_to_another_thread() {
    let (endpoint, _executor) = local_hub();
    std::thread::spawn(move || {
        assert_eq!(endpoint.role().unwrap(), Role::LocalHub);
        drop(endpoint);
    })
    .join()
    .unwrap();
}

#[test]
fn types_are_send_and_sync() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<Endpoint>();
    send_sync::<dxfeed_graal::Feed>();
    send_sync::<dxfeed_graal::Publisher>();
    send_sync::<ManualExecutor>();
}
