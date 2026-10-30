//! Endpoints: connections to data sources and hubs.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use dxfeed_graal_sys as sys;

use crate::error::{Error, Result, c_string};
use crate::executor::ManualExecutor;
use crate::feed::DXFeed;
use crate::handle::Handle;
use crate::isolate::{self, call};
use crate::publisher::DXPublisher;

/// The role of an endpoint (`DXEndpoint.Role`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Role {
    /// Receives data from a remote address with event conflation.
    Feed,
    /// Like `Feed`, for on-demand historical data.
    OnDemandFeed,
    /// Receives data without conflation.
    StreamFeed,
    /// Publishes data to a remote address or listens for subscribers.
    Publisher,
    /// Publishes data without conflation.
    StreamPublisher,
    /// A local hub: its publisher delivers events to its own feed, without a network.
    LocalHub,
}

impl Role {
    fn to_sys(self) -> sys::dxfg_v4_endpoint_role_t {
        match self {
            Role::Feed => sys::DXFG_ENDPOINT_ROLE_FEED,
            Role::OnDemandFeed => sys::DXFG_ENDPOINT_ROLE_ON_DEMAND_FEED,
            Role::StreamFeed => sys::DXFG_ENDPOINT_ROLE_STREAM_FEED,
            Role::Publisher => sys::DXFG_ENDPOINT_ROLE_PUBLISHER,
            Role::StreamPublisher => sys::DXFG_ENDPOINT_ROLE_STREAM_PUBLISHER,
            Role::LocalHub => sys::DXFG_ENDPOINT_ROLE_LOCAL_HUB,
        }
    }

    fn from_sys(role: sys::dxfg_v4_endpoint_role_t) -> Result<Self> {
        Ok(match role {
            sys::DXFG_ENDPOINT_ROLE_FEED => Role::Feed,
            sys::DXFG_ENDPOINT_ROLE_ON_DEMAND_FEED => Role::OnDemandFeed,
            sys::DXFG_ENDPOINT_ROLE_STREAM_FEED => Role::StreamFeed,
            sys::DXFG_ENDPOINT_ROLE_PUBLISHER => Role::Publisher,
            sys::DXFG_ENDPOINT_ROLE_STREAM_PUBLISHER => Role::StreamPublisher,
            sys::DXFG_ENDPOINT_ROLE_LOCAL_HUB => Role::LocalHub,
            other => return Err(Error::InvalidArgument(format!("unknown endpoint role {other}"))),
        })
    }
}

/// Builds a [`DXEndpoint`]; see [`DXEndpoint::new_builder`].
///
/// The settings are checked and passed to the SDK by [`build`](Self::build).
#[derive(Clone, Default)]
#[must_use]
pub struct DXEndpointBuilder {
    role: Option<Role>,
    name: Option<String>,
    properties: Vec<(String, String)>,
    executor: Option<ManualExecutor>,
}

impl DXEndpointBuilder {
    /// Sets the role; the default is [`Role::Feed`].
    pub fn with_role(mut self, role: Role) -> Self {
        self.role = Some(role);
        self
    }

    /// Sets the name used in logs and thread names (the [`DXEndpoint::NAME_PROPERTY`] property).
    ///
    /// Without a name the endpoint is named `qdrst`, and the next ones `qdrst-1`, `qdrst-2` and so on, as the other
    /// dxFeed Graal APIs name theirs `qdcxx` and `qdnet`.
    pub fn with_name(mut self, name: &str) -> Self {
        self.name = Some(name.to_owned());
        self
    }

    /// Sets an endpoint property, e.g. `dxfeed.aggregationPeriod`.
    pub fn with_property(mut self, key: &str, value: &str) -> Self {
        self.properties.push((key.to_owned(), value.to_owned()));
        self
    }

    /// Runs the tasks of the endpoint, listener calls included, on `executor`.
    ///
    /// Shutting the SDK down waits for those tasks, so keep processing the executor until then.
    pub fn with_executor(mut self, executor: &ManualExecutor) -> Self {
        self.executor = Some(executor.clone());
        self
    }

    /// Creates the endpoint.
    ///
    /// Returns [`Error::InvalidArgument`] if a name or property contains a NUL byte.
    pub fn build(self) -> Result<DXEndpoint> {
        let name = match self.name {
            Some(name) => Some(name),
            None if self.properties.iter().any(|(k, _)| k == DXEndpoint::NAME_PROPERTY) => None,
            None => Some(default_name(NEXT_DEFAULT_NAME.fetch_add(1, Ordering::Relaxed))),
        };
        let name = name.as_deref().map(|n| c_string(n, "endpoint name")).transpose()?;
        let properties = self
            .properties
            .iter()
            .map(|(k, v)| Ok((c_string(k, "property key")?, c_string(v, "property value")?)))
            .collect::<Result<Vec<_>>>()?;

        // SAFETY: the out-pointer is valid for writes.
        let builder = Handle::create(|t, out| unsafe { sys::dxfg_v4_DXEndpoint_newBuilder(t, out) })?;
        let b = builder.as_ptr();
        if let Some(role) = self.role {
            // SAFETY: the builder is live.
            call(|t| unsafe { sys::dxfg_v4_DXEndpoint_Builder_withRole(t, b, role.to_sys()) })?;
        }
        if let Some(name) = &name {
            // SAFETY: the builder is live and the name is NUL-terminated.
            call(|t| unsafe { sys::dxfg_v4_DXEndpoint_Builder_withName(t, b, name.as_ptr()) })?;
        }
        for (key, value) in &properties {
            // SAFETY: the builder is live and the strings are NUL-terminated.
            call(|t| unsafe { sys::dxfg_v4_DXEndpoint_Builder_withProperty(t, b, key.as_ptr(), value.as_ptr()) })?;
        }
        if let Some(executor) = &self.executor {
            let e = executor.handle.as_ptr();
            // SAFETY: the builder and the executor are live.
            call(|t| unsafe { sys::dxfg_v4_DXEndpoint_Builder_withExecutor(t, b, e) })?;
        }
        // SAFETY: the builder is live and the out-pointer is valid for writes.
        let handle = Handle::create(|t, out| unsafe { sys::dxfg_v4_DXEndpoint_Builder_build(t, b, out) })?;
        DXEndpoint::new(handle, true)
    }
}

/// The number of the next endpoint built without a name.
static NEXT_DEFAULT_NAME: AtomicU64 = AtomicU64::new(0);

/// The name of the endpoint built `number`-th without a name: `qdrst`, then `qdrst-1`, `qdrst-2` and so on.
fn default_name(number: u64) -> String {
    if number == 0 {
        "qdrst".to_owned()
    } else {
        format!("qdrst-{number}")
    }
}

/// What an endpoint, its feed and its publisher share; the endpoint is closed when the last of them is dropped.
pub(crate) struct EndpointState {
    endpoint: Handle<sys::dxfg_v4_endpoint_t>,
    pub(crate) feed: Handle<sys::dxfg_v4_feed_t>,
    pub(crate) publisher: Handle<sys::dxfg_v4_publisher_t>,
    role: Role,
    closed: AtomicBool,
}

impl EndpointState {
    fn close(&self) -> Result<()> {
        if self.closed.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        let e = self.endpoint.as_ptr();
        // SAFETY: the endpoint is live.
        call(|t| unsafe { sys::dxfg_v4_DXEndpoint_close(t, e) })
    }
}

impl Drop for EndpointState {
    fn drop(&mut self) {
        if !isolate::exiting() {
            let _ = self.close();
        }
    }
}

/// A connection to a data source or a local hub (`DXEndpoint`).
///
/// An endpoint, its [`DXFeed`], its [`DXPublisher`] and the subscriptions of the feed share it: cloning any of them is
/// cheap, and the endpoint stays open while one of them lives. Dropping the last one closes the endpoint, without
/// waiting for the notifications already scheduled. A chain such as
/// `let feed = DXEndpoint::create()?.connect("demo.dxfeed.com:7300")?.feed();` therefore keeps the endpoint open.
#[derive(Clone)]
pub struct DXEndpoint {
    state: Arc<EndpointState>,
}

impl std::fmt::Debug for DXEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DXEndpoint")
            .field("role", &self.state.role)
            .field("closed", &self.state.closed.load(Ordering::Acquire))
            .finish()
    }
}

impl DXEndpoint {
    /// Takes ownership of an endpoint and gets its role, feed and publisher, which every endpoint has.
    ///
    /// If that fails, the endpoint is closed when `close_on_failure`: an endpoint created for this call has no other
    /// owner to close it, but the default one of the process must stay open for the next attempt.
    pub(crate) fn new(endpoint: Handle<sys::dxfg_v4_endpoint_t>, close_on_failure: bool) -> Result<Self> {
        let e = endpoint.as_ptr();
        let parts = || {
            let mut role = sys::DXFG_ENDPOINT_ROLE_FEED;
            // SAFETY: the endpoint is live and the out-pointer is valid for writes.
            call(|t| unsafe { sys::dxfg_v4_DXEndpoint_getRole(t, e, &mut role) })?;
            // SAFETY: as above.
            let feed = Handle::create(|t, out| unsafe { sys::dxfg_v4_DXEndpoint_getFeed(t, e, out) })?;
            // SAFETY: as above.
            let publisher = Handle::create(|t, out| unsafe { sys::dxfg_v4_DXEndpoint_getPublisher(t, e, out) })?;
            Ok((Role::from_sys(role)?, feed, publisher))
        };
        let (role, feed, publisher) = parts().inspect_err(|_| {
            if close_on_failure {
                // SAFETY: the endpoint is live: its handle is released only after this call.
                let _ = call(|t| unsafe { sys::dxfg_v4_DXEndpoint_close(t, e) });
            }
        })?;
        Ok(DXEndpoint {
            state: Arc::new(EndpointState {
                endpoint,
                feed,
                publisher,
                role,
                closed: AtomicBool::new(false),
            }),
        })
    }

    /// The property that names an endpoint in logs and thread names (`DXEndpoint.NAME_PROPERTY`); see
    /// [`DXEndpointBuilder::with_name`].
    pub const NAME_PROPERTY: &'static str = "name";

    /// Returns a builder for an endpoint.
    #[doc(alias = "newBuilder")]
    pub fn new_builder() -> DXEndpointBuilder {
        DXEndpointBuilder::default()
    }

    /// Creates an endpoint with the [`Role::Feed`] role and default settings.
    pub fn create() -> Result<Self> {
        Self::create_with_role(Role::Feed)
    }

    /// Creates an endpoint with `role` and default settings, as `new_builder().with_role(role).build()` does.
    #[doc(alias = "create")]
    pub fn create_with_role(role: Role) -> Result<Self> {
        Self::new_builder().with_role(role).build()
    }

    /// Returns the default endpoint of the process, with the [`Role::Feed`] role (`DXEndpoint.getInstance()`).
    ///
    /// The endpoint is created on the first successful call and configured, as in Java, by the `dxfeed.properties` file
    /// and the system properties as of that moment (see [`system::set_property`](crate::system::set_property)): with
    /// `dxfeed.address` set it connects to that address. It is the endpoint of [`DXFeed::get_instance`]. All callers
    /// share it and it is never dropped: closing it closes it for the whole process.
    #[doc(alias = "getInstance")]
    pub fn get_instance() -> Result<&'static DXEndpoint> {
        static INSTANCE: OnceLock<DXEndpoint> = OnceLock::new();
        static INIT: Mutex<()> = Mutex::new(());
        crate::get_or_try_init(&INSTANCE, &INIT, || {
            // The SDK applies its defaults of the QD properties and registers the default endpoint, so that `shutdown`
            // closes it, only for the default feed: get the feed first.
            // SAFETY: the out-pointer is valid for writes.
            drop(Handle::create(|t, out| unsafe {
                sys::dxfg_v4_DXFeed_getInstance(t, out)
            })?);
            // SAFETY: as above.
            let handle = Handle::create(|t, out| unsafe { sys::dxfg_v4_DXEndpoint_getInstance(t, out) })?;
            Self::new(handle, false)
        })
    }

    /// Connects to `address`, e.g. `demo.dxfeed.com:7300`; the connection is made in the background.
    ///
    /// Returns the endpoint, so that calls chain.
    pub fn connect(&self, address: &str) -> Result<&Self> {
        let address = c_string(address, "address")?;
        let e = self.state.endpoint.as_ptr();
        // SAFETY: the endpoint is live and the address is NUL-terminated.
        call(|t| unsafe { sys::dxfg_v4_DXEndpoint_connect(t, e, address.as_ptr()) })?;
        Ok(self)
    }

    /// Disconnects; the endpoint can connect again.
    pub fn disconnect(&self) -> Result<()> {
        let e = self.state.endpoint.as_ptr();
        // SAFETY: the endpoint is live.
        call(|t| unsafe { sys::dxfg_v4_DXEndpoint_disconnect(t, e) })
    }

    /// Closes the endpoint for all its handles; it cannot be used afterwards. Closing again does nothing.
    ///
    /// Notifications already scheduled may still be delivered after it returns; see
    /// [`close_and_await_termination`](Self::close_and_await_termination).
    pub fn close(&self) -> Result<()> {
        self.state.close()
    }

    /// Closes the endpoint and waits until the notifications already scheduled are delivered. Calling it again, or
    /// after [`close`](Self::close), only waits.
    ///
    /// Inside a listener it returns [`Error::InCallback`]: it would wait for the notification that is running. With a
    /// [`ManualExecutor`], keep processing the executor from another thread until it returns.
    #[doc(alias = "closeAndAwaitTermination")]
    pub fn close_and_await_termination(&self) -> Result<()> {
        if isolate::in_callback() {
            return Err(Error::InCallback);
        }
        self.state.closed.store(true, Ordering::Release);
        let e = self.state.endpoint.as_ptr();
        // SAFETY: the endpoint is live.
        call(|t| unsafe { sys::dxfg_v4_DXEndpoint_closeAndAwaitTermination(t, e) })
    }

    /// The role of the endpoint.
    #[doc(alias = "getRole")]
    pub fn role(&self) -> Role {
        self.state.role
    }

    /// The feed of the endpoint; it keeps the endpoint open.
    #[doc(alias = "getFeed")]
    pub fn feed(&self) -> DXFeed {
        DXFeed::new(self.state.clone())
    }

    /// The publisher of the endpoint; it keeps the endpoint open.
    #[doc(alias = "getPublisher")]
    pub fn publisher(&self) -> DXPublisher {
        DXPublisher::new(self.state.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_names_count_from_the_second_endpoint() {
        assert_eq!(default_name(0), "qdrst");
        assert_eq!(default_name(1), "qdrst-1");
        assert_eq!(default_name(12), "qdrst-12");
    }
}
