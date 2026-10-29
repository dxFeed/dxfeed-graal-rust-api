//! Endpoints: connections to data sources and hubs.

use std::sync::atomic::{AtomicBool, Ordering};

use dxfeed_graal_sys as sys;

use crate::error::{Error, Result, c_string};
use crate::executor::ManualExecutor;
use crate::feed::Feed;
use crate::handle::Handle;
use crate::isolate::{self, call};
use crate::publisher::Publisher;

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

/// Builds an [`Endpoint`]; see [`Endpoint::builder`].
///
/// The settings are checked and passed to the SDK by [`build`](Self::build).
#[derive(Clone, Default)]
#[must_use]
pub struct EndpointBuilder {
    role: Option<Role>,
    name: Option<String>,
    properties: Vec<(String, String)>,
    executor: Option<ManualExecutor>,
}

impl EndpointBuilder {
    /// Sets the role; the default is [`Role::Feed`].
    pub fn with_role(mut self, role: Role) -> Self {
        self.role = Some(role);
        self
    }

    /// Sets the name used in logs and thread names.
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
    pub fn build(self) -> Result<Endpoint> {
        let name = self.name.as_deref().map(|n| c_string(n, "endpoint name")).transpose()?;
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
        Ok(Endpoint::new(handle))
    }
}

/// A connection to a data source or a local hub (`DXEndpoint`).
///
/// Dropping the endpoint closes it, without waiting for the notifications already scheduled.
pub struct Endpoint {
    handle: Handle<sys::dxfg_v4_endpoint_t>,
    closed: AtomicBool,
}

impl Endpoint {
    fn new(handle: Handle<sys::dxfg_v4_endpoint_t>) -> Self {
        Endpoint {
            handle,
            closed: AtomicBool::new(false),
        }
    }

    /// Returns a builder for an endpoint.
    pub fn builder() -> EndpointBuilder {
        EndpointBuilder::default()
    }

    /// Creates an endpoint with `role` and default settings.
    pub fn create(role: Role) -> Result<Self> {
        // SAFETY: the out-pointer is valid for writes.
        let handle = Handle::create(|t, out| unsafe { sys::dxfg_v4_DXEndpoint_createWithRole(t, role.to_sys(), out) })?;
        Ok(Endpoint::new(handle))
    }

    /// Connects to `address`, e.g. `demo.dxfeed.com:7300`; the connection is made in the background.
    pub fn connect(&self, address: &str) -> Result<()> {
        let address = c_string(address, "address")?;
        let e = self.handle.as_ptr();
        // SAFETY: the endpoint is live and the address is NUL-terminated.
        call(|t| unsafe { sys::dxfg_v4_DXEndpoint_connect(t, e, address.as_ptr()) })
    }

    /// Disconnects; the endpoint can connect again.
    pub fn disconnect(&self) -> Result<()> {
        let e = self.handle.as_ptr();
        // SAFETY: the endpoint is live.
        call(|t| unsafe { sys::dxfg_v4_DXEndpoint_disconnect(t, e) })
    }

    /// Closes the endpoint; it cannot be used afterwards. Closing again does nothing.
    ///
    /// Notifications already scheduled may still be delivered after it returns.
    pub fn close(&self) -> Result<()> {
        if self.closed.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        let e = self.handle.as_ptr();
        // SAFETY: the endpoint is live.
        call(|t| unsafe { sys::dxfg_v4_DXEndpoint_close(t, e) })
    }

    /// The role of the endpoint.
    pub fn role(&self) -> Result<Role> {
        let e = self.handle.as_ptr();
        let mut role = sys::DXFG_ENDPOINT_ROLE_FEED;
        // SAFETY: the endpoint is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_DXEndpoint_getRole(t, e, &mut role) })?;
        Role::from_sys(role)
    }

    /// The feed of the endpoint.
    pub fn feed(&self) -> Result<Feed> {
        let e = self.handle.as_ptr();
        // SAFETY: the endpoint is live and the out-pointer is valid for writes.
        let handle = Handle::create(|t, out| unsafe { sys::dxfg_v4_DXEndpoint_getFeed(t, e, out) })?;
        Ok(Feed { handle })
    }

    /// The publisher of the endpoint.
    pub fn publisher(&self) -> Result<Publisher> {
        let e = self.handle.as_ptr();
        // SAFETY: the endpoint is live and the out-pointer is valid for writes.
        let handle = Handle::create(|t, out| unsafe { sys::dxfg_v4_DXEndpoint_getPublisher(t, e, out) })?;
        Ok(Publisher { handle })
    }
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        if !isolate::exiting() {
            let _ = self.close();
        }
    }
}
