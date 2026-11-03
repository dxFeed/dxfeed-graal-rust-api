//! Endpoints: connections to data sources and hubs.

use std::collections::HashMap;
use std::fmt;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use dxfeed_graal_sys as sys;

use crate::error::{Error, Result, c_string};
use crate::event::EventType;
use crate::executor::Executor;
use crate::feed::DXFeed;
use crate::handle::Handle;
use crate::isolate::{self, call};
use crate::listener::{ChangeListeners, StateListener};
use crate::observable::event_types;
use crate::publisher::DXPublisher;
use crate::subscription::{ListenerId, check_aggregation_period};
use crate::time_period::{TimePeriod, TimePeriodInfo};

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

/// The state of an endpoint (`DXEndpoint.State`); see [`DXEndpoint::get_state`].
///
/// It prints as the Java constant: `NOT_CONNECTED`, `CONNECTING`, `CONNECTED` or `CLOSED`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum State {
    /// Not connected: just created, disconnected, or done reading its files.
    NotConnected,
    /// Connecting to its address.
    Connecting,
    /// Connected to its address, or listening for connections.
    Connected,
    /// Closed: it cannot be used anymore.
    Closed,
}

impl State {
    pub(crate) fn from_sys(state: sys::dxfg_endpoint_state_t) -> Result<Self> {
        Ok(match state {
            sys::DXFG_ENDPOINT_STATE_NOT_CONNECTED => State::NotConnected,
            sys::DXFG_ENDPOINT_STATE_CONNECTING => State::Connecting,
            sys::DXFG_ENDPOINT_STATE_CONNECTED => State::Connected,
            sys::DXFG_ENDPOINT_STATE_CLOSED => State::Closed,
            other => return Err(Error::InvalidArgument(format!("unknown endpoint state {other}"))),
        })
    }
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(match self {
            State::NotConnected => "NOT_CONNECTED",
            State::Connecting => "CONNECTING",
            State::Connected => "CONNECTED",
            State::Closed => "CLOSED",
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
    executor: Option<Executor>,
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

    /// Sets an endpoint property, such as [`DXEndpoint::DXFEED_AGGREGATION_PERIOD_PROPERTY`]; the endpoint ignores
    /// the properties it does not support (`withProperty`): see [`supports_property`](Self::supports_property).
    #[doc(alias = "withProperty")]
    pub fn with_property(mut self, key: &str, value: &str) -> Self {
        self.properties.push((key.to_owned(), value.to_owned()));
        self
    }

    /// Sets the properties of `properties` in their order, as [`with_property`](Self::with_property) does for each
    /// (`withProperties`).
    ///
    /// For a properties file, set its path with [`DXEndpoint::DXFEED_PROPERTIES_PROPERTY`] or
    /// [`DXEndpoint::DXPUBLISHER_PROPERTIES_PROPERTY`]: an endpoint with the [`Role::Feed`], [`Role::OnDemandFeed`] or
    /// [`Role::Publisher`] role reads it when it is built, and a missing file is skipped.
    #[doc(alias = "withProperties")]
    pub fn with_properties<K: AsRef<str>, V: AsRef<str>>(
        mut self,
        properties: impl IntoIterator<Item = (K, V)>,
    ) -> Self {
        for (key, value) in properties {
            self = self.with_property(key.as_ref(), value.as_ref());
        }
        self
    }

    /// Whether endpoints support the property `key` (`supportsProperty`); they ignore the others. Returns
    /// [`Error::InvalidArgument`] for a NUL byte.
    #[doc(alias = "supportsProperty")]
    pub fn supports_property(&self, key: &str) -> Result<bool> {
        let key = c_string(key, "property key")?;
        let builder = self.sdk_builder()?;
        let (b, mut supported) = (builder.as_ptr(), 0);
        // SAFETY: the builder is live, the key is NUL-terminated and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_DXEndpoint_Builder_supportsProperty(t, b, key.as_ptr(), &mut supported) })?;
        Ok(supported != 0)
    }

    /// A new SDK builder with the role of this builder.
    fn sdk_builder(&self) -> Result<Handle<sys::dxfg_v4_endpoint_builder_t>> {
        // SAFETY: the out-pointer is valid for writes.
        let builder = Handle::create(|t, out| unsafe { sys::dxfg_v4_DXEndpoint_newBuilder(t, out) })?;
        if let Some(role) = self.role {
            let b = builder.as_ptr();
            // SAFETY: the builder is live.
            call(|t| unsafe { sys::dxfg_v4_DXEndpoint_Builder_withRole(t, b, role.to_sys()) })?;
        }
        Ok(builder)
    }

    /// Runs the tasks of the endpoint, listener calls included, on `executor` (`withExecutor`), such as a thread pool
    /// or a [`ManualExecutor`](crate::ManualExecutor).
    ///
    /// Shutting the SDK down waits for those tasks, so keep processing a manual executor until then.
    #[doc(alias = "withExecutor")]
    pub fn with_executor(mut self, executor: &Executor) -> Self {
        self.executor = Some(executor.clone());
        self
    }

    /// Creates the endpoint.
    ///
    /// Returns [`Error::InvalidArgument`] if a name or property contains a NUL byte.
    pub fn build(self) -> Result<DXEndpoint> {
        let name = match &self.name {
            Some(name) => Some(name.clone()),
            None if self.properties.iter().any(|(k, _)| k == DXEndpoint::NAME_PROPERTY) => None,
            None => Some(default_name(NEXT_DEFAULT_NAME.fetch_add(1, Ordering::Relaxed))),
        };
        let name = name.as_deref().map(|n| c_string(n, "endpoint name")).transpose()?;
        let properties = self
            .properties
            .iter()
            .map(|(k, v)| Ok((c_string(k, "property key")?, c_string(v, "property value")?)))
            .collect::<Result<Vec<_>>>()?;

        let builder = self.sdk_builder()?;
        let b = builder.as_ptr();
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

/// The number of the roles; [`Role::LocalHub`] is the last.
const ROLES: usize = 6;
const _: () = assert!(Role::LocalHub as usize == ROLES - 1);

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
    state_listeners: Mutex<Vec<(ListenerId, StateListener)>>,
    /// The change listeners of the publisher's subscriptions by event type, kept until the endpoint is closed.
    publisher_listeners: Mutex<HashMap<EventType, Arc<ChangeListeners>>>,
}

impl EndpointState {
    /// The change listeners of the publisher's subscription to the events of `event_type`.
    pub(crate) fn publisher_listeners(&self, event_type: EventType) -> Arc<ChangeListeners> {
        crate::lock(&self.publisher_listeners)
            .entry(event_type)
            .or_insert_with(|| Arc::new(ChangeListeners::new("a change listener of this subscription")))
            .clone()
    }

    /// Lets go of the change listeners of the publisher's subscriptions, which the closed endpoint no longer calls:
    /// they are released once no [`ObservableSubscription`](crate::ObservableSubscription) holds them. This breaks
    /// the cycle of a listener that holds the publisher.
    fn release_publisher_listeners(&self) {
        let listeners = std::mem::take(&mut *crate::lock(&self.publisher_listeners));
        drop(listeners);
    }

    fn state_listeners(&self) -> MutexGuard<'_, Vec<(ListenerId, StateListener)>> {
        crate::lock(&self.state_listeners)
    }

    fn remove_state_listener(&self, listener: StateListener) -> Result<()> {
        let (e, l) = (self.endpoint.as_ptr(), listener.as_ptr());
        // SAFETY: the endpoint and the listener are live.
        let removed = call(|t| unsafe { sys::dxfg_v4_DXEndpoint_removeStateChangeListener(t, e, l) });
        listener.release();
        removed
    }

    fn close(&self) -> Result<()> {
        if self.closed.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        let e = self.endpoint.as_ptr();
        // SAFETY: the endpoint is live.
        let closed = call(|t| unsafe { sys::dxfg_v4_DXEndpoint_close(t, e) });
        self.release_publisher_listeners();
        closed
    }
}

impl Drop for EndpointState {
    fn drop(&mut self) {
        if isolate::exiting() {
            // No SDK calls from destructors while the process exits; the state listeners leak, as the SDK may still
            // call their trampoline.
            return;
        }
        // The listeners go first, so that no call of them starts once the last owner of the endpoint is gone.
        let listeners = std::mem::take(&mut *self.state_listeners());
        for (_, listener) in listeners {
            crate::error_manager::report_release("DXEndpoint", self.remove_state_listener(listener));
        }
        crate::error_manager::report_release("DXEndpoint", self.close());
    }
}

/// A connection to a data source or a local hub (`DXEndpoint`).
///
/// An endpoint, its [`DXFeed`], its [`DXPublisher`] and the subscriptions of the feed share it: cloning any of them is
/// cheap, and the endpoint stays open while one of them lives. Dropping the last one removes the state listeners,
/// waiting for their running calls as [`remove_state_listener`](Self::remove_state_listener) does, and closes the
/// endpoint without waiting for the notifications already scheduled. A chain such as
/// `let feed = DXEndpoint::create()?.connect("demo.dxfeed.com:7300")?.get_feed();` therefore keeps the endpoint open.
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
    /// The SDK endpoint, live while `self` is.
    pub(crate) fn as_ptr(&self) -> *mut sys::dxfg_v4_endpoint_t {
        self.state.endpoint.as_ptr()
    }

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
                state_listeners: Mutex::new(Vec::new()),
                publisher_listeners: Mutex::new(HashMap::new()),
            }),
        })
    }

    /// The property that names an endpoint in logs and thread names (`DXEndpoint.NAME_PROPERTY`); see
    /// [`DXEndpointBuilder::with_name`].
    pub const NAME_PROPERTY: &'static str = "name";

    /// The path of the properties file of an endpoint with the [`Role::Feed`] or [`Role::OnDemandFeed`] role
    /// (`DXEndpoint.DXFEED_PROPERTIES_PROPERTY`); without it, the `/dxfeed.properties` resource of the native image,
    /// if any.
    pub const DXFEED_PROPERTIES_PROPERTY: &'static str = "dxfeed.properties";

    /// The address that an endpoint with the [`Role::Feed`] role connects to when it is created, and one with the
    /// [`Role::OnDemandFeed`] role when it replays (`DXEndpoint.DXFEED_ADDRESS_PROPERTY`).
    pub const DXFEED_ADDRESS_PROPERTY: &'static str = "dxfeed.address";

    /// The user name of an endpoint with the [`Role::Feed`] or [`Role::OnDemandFeed`] role
    /// (`DXEndpoint.DXFEED_USER_PROPERTY`); see [`DXEndpoint::user`].
    pub const DXFEED_USER_PROPERTY: &'static str = "dxfeed.user";

    /// The password of an endpoint with the [`Role::Feed`] or [`Role::OnDemandFeed`] role
    /// (`DXEndpoint.DXFEED_PASSWORD_PROPERTY`); see [`DXEndpoint::password`].
    pub const DXFEED_PASSWORD_PROPERTY: &'static str = "dxfeed.password";

    /// The size of the thread pool of an endpoint with the [`Role::Feed`] role, the number of processors by default
    /// (`DXEndpoint.DXFEED_THREAD_POOL_SIZE_PROPERTY`).
    pub const DXFEED_THREAD_POOL_SIZE_PROPERTY: &'static str = "dxfeed.threadPoolSize";

    /// The aggregation period of an endpoint with the [`Role::Feed`] role, such as `0.1s`: a local limit of the rate
    /// of the notifications of the listeners, which does not change what the servers send
    /// (`DXEndpoint.DXFEED_AGGREGATION_PERIOD_PROPERTY`).
    pub const DXFEED_AGGREGATION_PERIOD_PROPERTY: &'static str = "dxfeed.aggregationPeriod";

    /// `true` turns on the support of wildcard symbols and of `tape:` addresses of a publisher
    /// (`DXEndpoint.DXFEED_WILDCARD_ENABLE_PROPERTY`).
    pub const DXFEED_WILDCARD_ENABLE_PROPERTY: &'static str = "dxfeed.wildcard.enable";

    /// The symbol striping strategy of an endpoint (`DXEndpoint.DXFEED_STRIPE_PROPERTY`).
    pub const DXFEED_STRIPE_PROPERTY: &'static str = "dxfeed.stripe";

    /// The sticky subscription period, in the format of [`TimePeriod`]
    /// (`DXEndpoint.DXFEED_STICKY_SUBSCRIPTION_PROPERTY`).
    pub const DXFEED_STICKY_SUBSCRIPTION_PROPERTY: &'static str = "dxfeed.stickySubscriptionPeriod";

    /// The aggregation period that the endpoint requests from the servers, in the format of [`TimePeriod`], such as
    /// `1s` or `0` for real time (`DXEndpoint.DXFEED_REQUESTED_AGGREGATION_PERIOD_PROPERTY`); see
    /// [`DXEndpoint::set_requested_aggregation_period`].
    pub const DXFEED_REQUESTED_AGGREGATION_PERIOD_PROPERTY: &'static str = "dxfeed.requestedAggregationPeriod";

    /// The path of the properties file of an endpoint with the [`Role::Publisher`] role
    /// (`DXEndpoint.DXPUBLISHER_PROPERTIES_PROPERTY`); without it, the `/dxpublisher.properties` resource of the
    /// native image, if any.
    pub const DXPUBLISHER_PROPERTIES_PROPERTY: &'static str = "dxpublisher.properties";

    /// The address that an endpoint with the [`Role::Publisher`] role connects to when it is created
    /// (`DXEndpoint.DXPUBLISHER_ADDRESS_PROPERTY`).
    pub const DXPUBLISHER_ADDRESS_PROPERTY: &'static str = "dxpublisher.address";

    /// The size of the thread pool of an endpoint with the [`Role::Publisher`] role, the number of processors by
    /// default (`DXEndpoint.DXPUBLISHER_THREAD_POOL_SIZE_PROPERTY`).
    pub const DXPUBLISHER_THREAD_POOL_SIZE_PROPERTY: &'static str = "dxpublisher.threadPoolSize";

    /// `true` turns on the event time of the events from a source that has it, such as a tape file or the on-demand
    /// service (`DXEndpoint.DXENDPOINT_EVENT_TIME_PROPERTY`).
    pub const DXENDPOINT_EVENT_TIME_PROPERTY: &'static str = "dxendpoint.eventTime";

    /// `true` stores all the lasting and indexed events, also without a subscription to them, with no means to remove
    /// them (`DXEndpoint.DXENDPOINT_STORE_EVERYTHING_PROPERTY`).
    pub const DXENDPOINT_STORE_EVERYTHING_PROPERTY: &'static str = "dxendpoint.storeEverything";

    /// `true` turns on the nanosecond precision of the business time (`DXEndpoint.DXSCHEME_NANO_TIME_PROPERTY`), as
    /// `dxscheme.enabled.Sequence=*` and `dxscheme.enabled.TimeNanoPart=*` do.
    pub const DXSCHEME_NANO_TIME_PROPERTY: &'static str = "dxscheme.nanoTime";

    /// The prefix of the properties that enable a field of the scheme in the events that match a glob, such as
    /// `dxscheme.enabled.TimeNanoPart=Trade` (`DXEndpoint.DXSCHEME_ENABLED_PROPERTY_PREFIX`).
    pub const DXSCHEME_ENABLED_PROPERTY_PREFIX: &'static str = "dxscheme.enabled.";

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

    /// Returns the default endpoint of the process with `role` (`DXEndpoint.getInstance(Role)`).
    ///
    /// It is [`get_instance`](Self::get_instance) for [`Role::Feed`]. For another role it is created on the first
    /// successful call, as [`create_with_role`](Self::create_with_role) does; one with the [`Role::OnDemandFeed`] or
    /// [`Role::Publisher`] role is configured by the properties file of the role and the system properties as of that
    /// moment, as in Java. All callers share it and it is never dropped: closing it closes it for the whole process.
    #[doc(alias = "getInstance")]
    pub fn get_instance_with_role(role: Role) -> Result<&'static DXEndpoint> {
        // One per role; the SDK has a default endpoint of the feed role only.
        static INSTANCES: [OnceLock<DXEndpoint>; ROLES] = [const { OnceLock::new() }; ROLES];
        static INIT: Mutex<()> = Mutex::new(());
        match role {
            Role::Feed => Self::get_instance(),
            _ => crate::get_or_try_init(&INSTANCES[role as usize], &INIT, || Self::create_with_role(role)),
        }
    }

    /// Changes the user name of the endpoint (`user`); call it before [`connect`](Self::connect), with
    /// [`password`](Self::password), for a service that needs credentials.
    ///
    /// Returns the endpoint, so that calls chain, and [`Error::InvalidArgument`] for a NUL byte.
    pub fn user(&self, user: &str) -> Result<&Self> {
        let user = c_string(user, "user name")?;
        let e = self.as_ptr();
        // SAFETY: the endpoint is live and the SDK copies the string.
        call(|t| unsafe { sys::dxfg_v4_DXEndpoint_user(t, e, user.as_ptr()) })?;
        Ok(self)
    }

    /// Changes the password of the endpoint (`password`); see [`user`](Self::user).
    pub fn password(&self, password: &str) -> Result<&Self> {
        let password = c_string(password, "password")?;
        let e = self.as_ptr();
        // SAFETY: the endpoint is live and the SDK copies the string.
        call(|t| unsafe { sys::dxfg_v4_DXEndpoint_password(t, e, password.as_ptr()) })?;
        Ok(self)
    }

    /// The event types that the endpoint supports (`getEventTypes`), without those this crate does not know.
    #[doc(alias = "getEventTypes")]
    pub fn get_event_types(&self) -> Result<Vec<EventType>> {
        let (e, mut list) = (self.as_ptr(), ptr::null_mut());
        // SAFETY: the endpoint is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_DXEndpoint_getEventTypes(t, e, &mut list) })?;
        // SAFETY: the SDK returned a live list of `size` live classes, or NULL.
        let types = unsafe { event_types(list) };
        // SAFETY: the list came from the SDK and is released once.
        let released = call(|t| unsafe { sys::dxfg_v4_event_clazz_list_release(t, list) });
        crate::error_manager::report_release("DXEndpoint", released);
        Ok(types)
    }

    /// The aggregation period that the endpoint requests from the servers; `None` if it requests none
    /// (`getRequestedAggregationPeriod`).
    #[doc(alias = "getRequestedAggregationPeriod")]
    pub fn get_requested_aggregation_period(&self) -> Result<Option<TimePeriod>> {
        let e = self.as_ptr();
        // SAFETY: the endpoint is live and the out-pointer is valid for writes.
        let period = Handle::create_optional(|t, out| unsafe {
            sys::dxfg_v4_DXEndpoint_getRequestedAggregationPeriod(t, e, out)
        })?;
        period.as_ref().map(TimePeriod::from_sdk).transpose()
    }

    /// Requests an aggregation period from the servers, which keep it within their own bounds and apply it on their
    /// side (`setRequestedAggregationPeriod`): [`TimePeriod::ZERO`] requests real-time delivery and `None` clears the
    /// request. [`get_aggregation_period_info`](Self::get_aggregation_period_info) tells the period they apply.
    ///
    /// Returns [`Error::InvalidArgument`] for a period longer than
    /// [`AGGREGATION_PERIOD_MAX_VALUE`](crate::DXFeedSubscription::AGGREGATION_PERIOD_MAX_VALUE).
    #[doc(alias = "setRequestedAggregationPeriod")]
    pub fn set_requested_aggregation_period(&self, period: Option<TimePeriod>) -> Result<()> {
        check_aggregation_period(period)?;
        let period = period.map(TimePeriod::to_sdk).transpose()?;
        let e = self.as_ptr();
        let p = period.as_ref().map_or(ptr::null_mut(), Handle::as_ptr);
        // SAFETY: the endpoint and the period, if any, are live.
        call(|t| unsafe { sys::dxfg_v4_DXEndpoint_setRequestedAggregationPeriod(t, e, p) })
    }

    /// The aggregation period that the servers apply, over all the connections of the endpoint
    /// (`getAggregationPeriodInfo`). It is [`TimePeriodInfo::UNKNOWN`] until a server reports it, and when none does,
    /// such as for a local hub or a publisher.
    #[doc(alias = "getAggregationPeriodInfo")]
    pub fn get_aggregation_period_info(&self) -> Result<TimePeriodInfo> {
        let (e, mut min, mut max) = (self.as_ptr(), 0, 0);
        // SAFETY: the endpoint is live and the out-pointers are valid for writes.
        call(|t| unsafe { sys::dxfg_v4_DXEndpoint_getAggregationPeriodInfo(t, e, &mut min, &mut max) })?;
        if min < 0 || max < 0 {
            return Ok(TimePeriodInfo::UNKNOWN);
        }
        TimePeriodInfo::value_of(min, max)
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

    /// Drops the connection and connects again to the same address; does nothing if the endpoint is not connected or
    /// connecting.
    pub fn reconnect(&self) -> Result<()> {
        let e = self.state.endpoint.as_ptr();
        // SAFETY: the endpoint is live.
        call(|t| unsafe { sys::dxfg_v4_DXEndpoint_reconnect(t, e) })
    }

    /// Disconnects; the endpoint can connect again.
    pub fn disconnect(&self) -> Result<()> {
        let e = self.state.endpoint.as_ptr();
        // SAFETY: the endpoint is live.
        call(|t| unsafe { sys::dxfg_v4_DXEndpoint_disconnect(t, e) })
    }

    /// Disconnects and clears the data stored in the endpoint: the last events of its feed and the events its
    /// publisher has not sent yet. The endpoint can connect again.
    #[doc(alias = "disconnectAndClear")]
    pub fn disconnect_and_clear(&self) -> Result<()> {
        let e = self.state.endpoint.as_ptr();
        // SAFETY: the endpoint is live.
        call(|t| unsafe { sys::dxfg_v4_DXEndpoint_disconnectAndClear(t, e) })
    }

    /// Waits until the endpoint is [`State::NotConnected`] or [`State::Closed`]: for a `file:` address, until the file
    /// is read. The listeners may still be processing its events; [`close_and_await_termination`] waits for them.
    ///
    /// Inside a listener it returns [`Error::InCallback`]: it could wait for the notification that is running. With
    /// a [`ManualExecutor`](crate::ManualExecutor), keep processing the executor from another thread until it returns.
    ///
    /// [`close_and_await_termination`]: Self::close_and_await_termination
    #[doc(alias = "awaitNotConnected")]
    pub fn await_not_connected(&self) -> Result<()> {
        if isolate::in_callback() {
            return Err(Error::InCallback);
        }
        let e = self.state.endpoint.as_ptr();
        // SAFETY: the endpoint is live.
        call(|t| unsafe { sys::dxfg_v4_DXEndpoint_awaitNotConnected(t, e) })
    }

    /// Waits until the endpoint has processed the data received or published so far: for a `tape:` address, until
    /// the published events are written, so call it before closing the endpoint.
    ///
    /// Inside a listener it returns [`Error::InCallback`], as [`await_not_connected`](Self::await_not_connected) does.
    #[doc(alias = "awaitProcessed")]
    pub fn await_processed(&self) -> Result<()> {
        if isolate::in_callback() {
            return Err(Error::InCallback);
        }
        let e = self.state.endpoint.as_ptr();
        // SAFETY: the endpoint is live.
        call(|t| unsafe { sys::dxfg_v4_DXEndpoint_awaitProcessed(t, e) })
    }

    /// The state of the endpoint.
    #[doc(alias = "getState")]
    pub fn get_state(&self) -> Result<State> {
        let e = self.state.endpoint.as_ptr();
        let mut state = sys::DXFG_ENDPOINT_STATE_NOT_CONNECTED;
        // SAFETY: the endpoint is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_DXEndpoint_getState(t, e, &mut state) })?;
        State::from_sys(state)
    }

    /// Adds a listener of state changes; `f` is called with the old and the new state, on a thread of the endpoint's
    /// executor.
    ///
    /// Fast changes are coalesced: `f` may see [`State::NotConnected`] followed by [`State::Connected`]. A panic in `f`
    /// is caught and goes to [`ErrorManager`](crate::ErrorManager); the listener stays active. The listener stays
    /// until it is removed or the last owner of the endpoint is dropped; a closure that owns the endpoint, its feed
    /// or its publisher keeps it open.
    #[doc(alias = "addStateChangeListener")]
    pub fn add_state_listener(&self, f: impl FnMut(State, State) + Send + 'static) -> Result<ListenerId> {
        let listener = StateListener::new(Box::new(f))?;
        let (e, l) = (self.state.endpoint.as_ptr(), listener.as_ptr());
        // SAFETY: the endpoint and the listener are live.
        if let Err(error) = call(|t| unsafe { sys::dxfg_v4_DXEndpoint_addStateChangeListener(t, e, l) }) {
            listener.release();
            return Err(error);
        }
        let id = ListenerId::next();
        self.state.state_listeners().push((id, listener));
        Ok(id)
    }

    /// Removes a state listener. Called outside listeners, it returns once no call of the listener runs; called from a
    /// listener, it returns at once and the listener is released in the background. So do not hold a lock that the
    /// closure takes while removing it from another thread.
    ///
    /// Returns [`Error::InvalidArgument`] if the listener is not a state listener of this endpoint.
    #[doc(alias = "removeStateChangeListener")]
    pub fn remove_state_listener(&self, id: ListenerId) -> Result<()> {
        let listener = {
            let mut listeners = self.state.state_listeners();
            let index = listeners
                .iter()
                .position(|(i, _)| *i == id)
                .ok_or_else(|| Error::InvalidArgument(format!("{id:?} is not a state listener of this endpoint")))?;
            listeners.swap_remove(index).1
        };
        self.state.remove_state_listener(listener)
    }

    /// Closes the endpoint for all its handles; it cannot be used afterwards. Closing again does nothing.
    ///
    /// Notifications already scheduled may still be delivered after it returns; see
    /// [`close_and_await_termination`](Self::close_and_await_termination). Dropping the last owner of the endpoint
    /// right after `close` removes the state listeners before they see [`State::Closed`]; to see it, use
    /// `close_and_await_termination`.
    pub fn close(&self) -> Result<()> {
        self.state.close()
    }

    /// Closes the endpoint and waits until the notifications already scheduled are delivered. Calling it again, or
    /// after [`close`](Self::close), only waits.
    ///
    /// Inside a listener it returns [`Error::InCallback`]: it would wait for the notification that is running. With a
    /// [`ManualExecutor`](crate::ManualExecutor), keep processing the executor from another thread until it returns.
    /// It also waits while an [event queue](crate::event_queue) of a subscription of the endpoint holds events that no
    /// poll returned, and while the executor of a queue that a subscription has as its executor holds work: close the
    /// queue first, and never call it on the polling thread (see
    /// [`SharedEventQueue::get_executor`](crate::event_queue::SharedEventQueue::get_executor)).
    #[doc(alias = "closeAndAwaitTermination")]
    pub fn close_and_await_termination(&self) -> Result<()> {
        if isolate::in_callback() {
            return Err(Error::InCallback);
        }
        self.state.closed.store(true, Ordering::Release);
        let e = self.state.endpoint.as_ptr();
        // SAFETY: the endpoint is live.
        let closed = call(|t| unsafe { sys::dxfg_v4_DXEndpoint_closeAndAwaitTermination(t, e) });
        self.state.release_publisher_listeners();
        closed
    }

    /// The role of the endpoint.
    #[doc(alias = "getRole")]
    pub fn get_role(&self) -> Role {
        self.state.role
    }

    /// The feed of the endpoint; it keeps the endpoint open.
    #[doc(alias = "getFeed")]
    pub fn get_feed(&self) -> DXFeed {
        DXFeed::new(self.state.clone())
    }

    /// The publisher of the endpoint; it keeps the endpoint open.
    #[doc(alias = "getPublisher")]
    pub fn get_publisher(&self) -> DXPublisher {
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
