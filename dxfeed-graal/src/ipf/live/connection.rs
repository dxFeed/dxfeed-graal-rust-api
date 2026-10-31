//! The connection that fills a collector from a file or a service of instrument profiles.

use std::fmt;
use std::ptr;
use std::time::Duration;

use dxfeed_graal_sys as sys;

use crate::error::{Error, Result, c_string};
use crate::handle::Handle;
use crate::ipf::live::InstrumentProfileCollector;
use crate::ipf::profile::take_string;
use crate::isolate::{self, call};
use crate::listener::{IpfStateListener, IpfStateListeners};
use crate::subscription::ListenerId;

/// The state of an [`InstrumentProfileConnection`] (`InstrumentProfileConnection.State`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConnectionState {
    /// Created, not started yet.
    NotConnected,
    /// Started, connecting to the address.
    Connecting,
    /// Connected, reading the profiles.
    Connected,
    /// Read the profiles completely at least once.
    Completed,
    /// Closed.
    Closed,
}

impl ConnectionState {
    pub(crate) fn from_sys(state: sys::dxfg_v4_ipf_connection_state_t) -> Result<Self> {
        Ok(match state {
            sys::DXFG_IPF_CONNECTION_STATE_NOT_CONNECTED => ConnectionState::NotConnected,
            sys::DXFG_IPF_CONNECTION_STATE_CONNECTING => ConnectionState::Connecting,
            sys::DXFG_IPF_CONNECTION_STATE_CONNECTED => ConnectionState::Connected,
            sys::DXFG_IPF_CONNECTION_STATE_COMPLETED => ConnectionState::Completed,
            sys::DXFG_IPF_CONNECTION_STATE_CLOSED => ConnectionState::Closed,
            other => return Err(Error::InvalidArgument(format!("unknown connection state {other}"))),
        })
    }
}

impl fmt::Display for ConnectionState {
    /// The name of the state in Java, such as `NOT_CONNECTED`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ConnectionState::NotConnected => "NOT_CONNECTED",
            ConnectionState::Connecting => "CONNECTING",
            ConnectionState::Connected => "CONNECTED",
            ConnectionState::Completed => "COMPLETED",
            ConnectionState::Closed => "CLOSED",
        })
    }
}

/// Reads instrument profiles from a file or a service into a collector, in the background
/// (`InstrumentProfileConnection`).
///
/// A service with live updates sends the changes as they come; a file is read again every
/// [`update_period`](Self::update_period) if it changed. Dropping the connection removes its state listeners and
/// closes it.
///
/// ```no_run
/// use std::time::Duration;
///
/// use dxfeed_graal::ipf::live::{InstrumentProfileCollector, InstrumentProfileConnection};
///
/// let collector = InstrumentProfileCollector::new()?;
/// let connection = InstrumentProfileConnection::create_connection("profiles.ipf.zip", &collector)?;
/// connection.start()?;
/// connection.wait_until_completed(Duration::from_secs(10))?;
/// println!("{} profiles", collector.view()?.len());
/// # Ok::<(), dxfeed_graal::Error>(())
/// ```
pub struct InstrumentProfileConnection {
    handle: Handle<sys::dxfg_v4_ipf_connection_t>,
    listeners: IpfStateListeners,
    _collector: InstrumentProfileCollector,
}

impl fmt::Debug for InstrumentProfileConnection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InstrumentProfileConnection").finish_non_exhaustive()
    }
}

impl Drop for InstrumentProfileConnection {
    fn drop(&mut self) {
        if isolate::exiting() {
            // No SDK calls from destructors while the process exits; the listeners' state is leaked.
            return;
        }
        let c = self.handle.as_ptr();
        self.listeners.remove_all(|l| {
            // SAFETY: the connection and the listener are live.
            call(|t| unsafe { sys::dxfg_v4_InstrumentProfileConnection_removeStateChangeListener(t, c, l) })
        });
        let _ = self.close();
    }
}

impl InstrumentProfileConnection {
    /// Creates a connection that reads the profiles from `address` into `collector` once it is started
    /// (`createConnection`).
    ///
    /// The address is a file path, a URL such as `https://user:password@tools.dxfeed.com/ipf` or `<host>:<port>`.
    #[doc(alias = "createConnection")]
    pub fn create_connection(address: &str, collector: &InstrumentProfileCollector) -> Result<Self> {
        let address = c_string(address, "the address")?;
        let c = collector.as_ptr();
        // SAFETY: the address and the collector are live; the out-pointer is valid for writes.
        let handle = Handle::create(|t, out| unsafe {
            sys::dxfg_v4_InstrumentProfileConnection_createConnection(t, address.as_ptr(), c, out)
        })?;
        Ok(InstrumentProfileConnection {
            handle,
            listeners: IpfStateListeners::new("a state listener of this connection"),
            _collector: collector.clone(),
        })
    }

    /// The address of the connection (`getAddress`).
    #[doc(alias = "getAddress")]
    pub fn address(&self) -> Result<String> {
        let mut address = ptr::null();
        let c = self.handle.as_ptr();
        // SAFETY: the connection is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_InstrumentProfileConnection_getAddress(t, c, &mut address) })?;
        // SAFETY: the SDK wrote a NUL-terminated string or NULL, which is ours to release.
        Ok(unsafe { take_string(address) })
    }

    /// How often a file is checked for changes and read again (`getUpdatePeriod`).
    #[doc(alias = "getUpdatePeriod")]
    pub fn update_period(&self) -> Result<Duration> {
        let mut millis = 0;
        let c = self.handle.as_ptr();
        // SAFETY: the connection is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_InstrumentProfileConnection_getUpdatePeriod(t, c, &mut millis) })?;
        Ok(Duration::from_millis(millis.max(0) as u64))
    }

    /// Sets how often a file is checked for changes, in whole milliseconds (`setUpdatePeriod`).
    #[doc(alias = "setUpdatePeriod")]
    pub fn set_update_period(&self, period: Duration) -> Result<()> {
        let millis = i64::try_from(period.as_millis()).unwrap_or(i64::MAX);
        let c = self.handle.as_ptr();
        // SAFETY: the connection is live.
        call(|t| unsafe { sys::dxfg_v4_InstrumentProfileConnection_setUpdatePeriod(t, c, millis) })
    }

    /// The state of the connection (`getState`).
    #[doc(alias = "getState")]
    pub fn state(&self) -> Result<ConnectionState> {
        let mut state = sys::DXFG_IPF_CONNECTION_STATE_NOT_CONNECTED;
        let c = self.handle.as_ptr();
        // SAFETY: the connection is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_InstrumentProfileConnection_getState(t, c, &mut state) })?;
        ConnectionState::from_sys(state)
    }

    /// The modification time of the last profiles read, in milliseconds since the Unix epoch, or `0` if none were
    /// read yet (`getLastModified`). It is updated just after the state becomes
    /// [`Completed`](ConnectionState::Completed).
    #[doc(alias = "getLastModified")]
    pub fn last_modified(&self) -> Result<i64> {
        let mut time = 0;
        let c = self.handle.as_ptr();
        // SAFETY: the connection is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_InstrumentProfileConnection_getLastModified(t, c, &mut time) })?;
        Ok(time)
    }

    /// Starts the connection in the background (`start`).
    ///
    /// Returns an error if the connection is not in the [`NotConnected`](ConnectionState::NotConnected) state.
    pub fn start(&self) -> Result<()> {
        let c = self.handle.as_ptr();
        // SAFETY: the connection is live.
        call(|t| unsafe { sys::dxfg_v4_InstrumentProfileConnection_start(t, c) })
    }

    /// Closes the connection; the collector keeps the profiles read (`close`).
    pub fn close(&self) -> Result<()> {
        let c = self.handle.as_ptr();
        // SAFETY: the connection is live.
        call(|t| unsafe { sys::dxfg_v4_InstrumentProfileConnection_close(t, c) })
    }

    /// Adds a listener of the state changes, which receives the old and the new state, and returns its id
    /// (`addStateChangeListener`).
    ///
    /// Calls of a listener do not overlap. A change that the listener makes itself, such as [`close`](Self::close)
    /// from its call, is not told to it: that call would have to wait for its own end.
    #[doc(alias = "addStateChangeListener")]
    pub fn add_state_change_listener(
        &self,
        listener: impl FnMut(ConnectionState, ConnectionState) + Send + 'static,
    ) -> Result<ListenerId> {
        let c = self.handle.as_ptr();
        self.listeners.add(IpfStateListener::new(Box::new(listener))?, |l| {
            // SAFETY: the connection and the listener are live.
            call(|t| unsafe { sys::dxfg_v4_InstrumentProfileConnection_addStateChangeListener(t, c, l) })
        })
    }

    /// Removes a state listener, waiting for its running call unless called from it (`removeStateChangeListener`).
    ///
    /// Returns [`Error::InvalidArgument`] if the listener is not in this connection.
    #[doc(alias = "removeStateChangeListener")]
    pub fn remove_state_change_listener(&self, id: ListenerId) -> Result<()> {
        let c = self.handle.as_ptr();
        self.listeners.remove(id, |l| {
            // SAFETY: the connection and the listener are live.
            call(|t| unsafe { sys::dxfg_v4_InstrumentProfileConnection_removeStateChangeListener(t, c, l) })
        })
    }

    /// Waits until the profiles are read completely, the connection is closed or the timeout passes; returns
    /// whether they were read (`waitUntilCompleted`).
    ///
    /// Inside a listener it returns [`Error::InCallback`], as it could wait for that listener.
    #[doc(alias = "waitUntilCompleted")]
    pub fn wait_until_completed(&self, timeout: Duration) -> Result<bool> {
        if isolate::in_callback() {
            return Err(Error::InCallback);
        }
        let millis = i64::try_from(timeout.as_millis()).unwrap_or(i64::MAX);
        let mut completed = 0;
        let c = self.handle.as_ptr();
        // SAFETY: the connection is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_InstrumentProfileConnection_waitUntilCompleted(t, c, millis, &mut completed) })?;
        Ok(completed != 0)
    }
}
