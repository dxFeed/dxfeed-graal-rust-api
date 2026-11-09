//! The message connectors of an endpoint (`MessageConnector`); see [`DXEndpoint::get_connectors`].
//!
//! [`DXEndpoint::get_connectors`]: crate::DXEndpoint::get_connectors

use std::fmt;
use std::sync::Arc;

use dxfeed_graal_sys as sys;

use crate::error::{Error, JavaException, Result};
use crate::handle::Handle;
use crate::isolate::{self, call};

/// The kind of a [`MessageConnector`]; see [`MessageConnector::get_kind`].
///
/// It prints as the SDK constant: `CLIENT_SOCKET`, `SERVER_SOCKET` or `OTHER`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MessageConnectorKind {
    /// A client socket connector: an address `host:port`.
    ClientSocket,
    /// A server socket connector: an address `:port`.
    ServerSocket,
    /// Any other connector, such as a file or a custom one, and a kind that a later SDK adds.
    Other,
}

impl MessageConnectorKind {
    fn from_sys(kind: i32) -> Self {
        match u32::try_from(kind) {
            Ok(sys::DXFG_V4_MESSAGE_CONNECTOR_KIND_CLIENT_SOCKET) => MessageConnectorKind::ClientSocket,
            Ok(sys::DXFG_V4_MESSAGE_CONNECTOR_KIND_SERVER_SOCKET) => MessageConnectorKind::ServerSocket,
            _ => MessageConnectorKind::Other,
        }
    }
}

impl fmt::Display for MessageConnectorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(match self {
            MessageConnectorKind::ClientSocket => "CLIENT_SOCKET",
            MessageConnectorKind::ServerSocket => "SERVER_SOCKET",
            MessageConnectorKind::Other => "OTHER",
        })
    }
}

/// The state of a [`MessageConnector`] (`MessageConnectorState`); see [`MessageConnector::get_state`].
///
/// It prints as the Java constant: `CONNECTING`, `CONNECTED` or `DISCONNECTED`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MessageConnectorState {
    /// Started and not connected yet, or reconnecting after a failure or a drop.
    Connecting,
    /// At least one of its connections is established; a server socket connector accepts connections.
    Connected,
    /// Stopped: replaced by a connect of its endpoint to another address, or its endpoint disconnected or closed.
    Disconnected,
}

impl MessageConnectorState {
    fn from_sys(state: i32) -> Result<Self> {
        Ok(match u32::try_from(state) {
            Ok(sys::DXFG_V4_MESSAGE_CONNECTOR_STATE_CONNECTING) => MessageConnectorState::Connecting,
            Ok(sys::DXFG_V4_MESSAGE_CONNECTOR_STATE_CONNECTED) => MessageConnectorState::Connected,
            Ok(sys::DXFG_V4_MESSAGE_CONNECTOR_STATE_DISCONNECTED) => MessageConnectorState::Disconnected,
            _ => {
                return Err(Error::InvalidArgument(format!(
                    "unknown message connector state {state}"
                )));
            }
        })
    }
}

impl fmt::Display for MessageConnectorState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(match self {
            MessageConnectorState::Connecting => "CONNECTING",
            MessageConnectorState::Connected => "CONNECTED",
            MessageConnectorState::Disconnected => "DISCONNECTED",
        })
    }
}

/// The statistics of a [`MessageConnector`] at one moment; see [`MessageConnector::get_statistics`].
///
/// The counters cover all the connections of the connector since it was created, the closed ones included. They are
/// kept apart from the other values of the connector: `active_connection_count` may follow
/// [`MessageConnector::get_connection_count`] a moment later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub struct MessageConnectorStatistics {
    /// The bytes read, heartbeats included: they grow on an idle connection too.
    pub read_bytes: i64,
    /// The bytes written, heartbeats included.
    pub written_bytes: i64,
    /// The data records read: they grow only when data arrives.
    pub read_data_records: i64,
    /// The data records written.
    pub written_data_records: i64,
    /// The open connections.
    pub active_connection_count: i64,
    /// The connections closed since the connector was created.
    pub closed_connection_count: i64,
}

/// The category of a [`ConnectionError`].
///
/// It prints as the SDK constant without its prefix: `OTHER`, `DNS`, `CONNECT_FAILED`, `READ_TIMEOUT`,
/// `HEARTBEAT_TIMEOUT`, `CLOSED_BY_REMOTE`, `TLS` or `IO`.
#[cfg(feature = "experimental")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ConnectionErrorCategory {
    /// None of the others, such as an error of the protocol or of the authentication, and a category that a later
    /// SDK adds.
    Other,
    /// The host name of the address is not resolved (`java.net.UnknownHostException`).
    Dns,
    /// The connect failed, was refused or timed out (`java.net.ConnectException` and others).
    ConnectFailed,
    /// A read of an established connection timed out (`java.net.SocketTimeoutException`).
    ReadTimeout,
    /// The remote side sent no data and no heartbeat within the heartbeat timeout (the address option
    /// `[heartbeatTimeout=...]`), so QD closed the connection; the error has no exception.
    HeartbeatTimeout,
    /// The remote side closed the connection (`java.io.EOFException`).
    ClosedByRemote,
    /// The TLS handshake or a TLS record failed, such as a `tls+` address of a port without TLS or a certificate that
    /// is not trusted (`javax.net.ssl.SSLException`).
    Tls,
    /// Another I/O error, such as a connection reset (`java.io.IOException`).
    Io,
}

#[cfg(feature = "experimental")]
impl ConnectionErrorCategory {
    fn from_sys(category: i32) -> Self {
        match u32::try_from(category) {
            Ok(sys::DXFG_V4_CONNECTION_ERROR_DNS) => ConnectionErrorCategory::Dns,
            Ok(sys::DXFG_V4_CONNECTION_ERROR_CONNECT_FAILED) => ConnectionErrorCategory::ConnectFailed,
            Ok(sys::DXFG_V4_CONNECTION_ERROR_READ_TIMEOUT) => ConnectionErrorCategory::ReadTimeout,
            Ok(sys::DXFG_V4_CONNECTION_ERROR_HEARTBEAT_TIMEOUT) => ConnectionErrorCategory::HeartbeatTimeout,
            Ok(sys::DXFG_V4_CONNECTION_ERROR_CLOSED_BY_REMOTE) => ConnectionErrorCategory::ClosedByRemote,
            Ok(sys::DXFG_V4_CONNECTION_ERROR_TLS) => ConnectionErrorCategory::Tls,
            Ok(sys::DXFG_V4_CONNECTION_ERROR_IO) => ConnectionErrorCategory::Io,
            _ => ConnectionErrorCategory::Other,
        }
    }
}

#[cfg(feature = "experimental")]
impl fmt::Display for ConnectionErrorCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(match self {
            ConnectionErrorCategory::Other => "OTHER",
            ConnectionErrorCategory::Dns => "DNS",
            ConnectionErrorCategory::ConnectFailed => "CONNECT_FAILED",
            ConnectionErrorCategory::ReadTimeout => "READ_TIMEOUT",
            ConnectionErrorCategory::HeartbeatTimeout => "HEARTBEAT_TIMEOUT",
            ConnectionErrorCategory::ClosedByRemote => "CLOSED_BY_REMOTE",
            ConnectionErrorCategory::Tls => "TLS",
            ConnectionErrorCategory::Io => "IO",
        })
    }
}

/// A connection error of a [`MessageConnector`]: a failed connect or a dropped connection; see
/// [`MessageConnector::get_last_error`] and [`DXEndpoint::get_last_connection_error`].
///
/// It is a copy, which does not change when the connector records a newer error. A successful connection clears
/// nothing, so compare [`time`](Self::time) or [`count`](Self::count) with an error seen before to tell a new error
/// from an old one.
///
/// The SDK captures the errors from the log records of QD's default logging, at any `log.level` and with or without a
/// [logging listener](crate::logging). With another `log.className` nothing is captured, and a level set on QD's
/// loggers `com.devexperts.qd.qtp.MessageConnector`, `com.devexperts.qd.qtp.MessageAdapterConnection` or
/// `com.devexperts.connector.codec.ssl` themselves may stop the capture.
///
/// [`DXEndpoint::get_last_connection_error`]: crate::DXEndpoint::get_last_connection_error
#[cfg(feature = "experimental")]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct ConnectionError {
    /// The time of the error, in milliseconds since the epoch (UTC).
    pub time: i64,
    /// The address the connector was configured with, the user and the password masked: the whole list of a
    /// multi-host address, not the host that failed.
    pub address: String,
    /// The category of the error.
    pub category: ConnectionErrorCategory,
    /// Whether an established connection was dropped, as opposed to a failed connect attempt.
    pub dropped: bool,
    /// The class name of the Java exception, such as `java.net.ConnectException`; `None` when there is none, as for
    /// [`HeartbeatTimeout`](ConnectionErrorCategory::HeartbeatTimeout).
    pub exception_class_name: Option<String>,
    /// The message of the exception, or of QD's log record when there is no exception, the user and the password
    /// masked, at most 256 characters; `None` when there is none.
    pub message: Option<String>,
    /// The number of errors of the connector up to this one.
    pub count: i64,
}

#[cfg(feature = "experimental")]
impl ConnectionError {
    /// Calls an SDK function that writes a new error or NULL to its out-pointer, copies the error and releases it.
    pub(crate) fn last(
        f: impl FnOnce(isolate::Thread, *mut *mut sys::dxfg_v4_connection_error_t) -> i32,
    ) -> Result<Option<Self>> {
        let mut error = std::ptr::null_mut();
        call(|t| f(t, &mut error))?;
        if error.is_null() {
            return Ok(None);
        }
        // SAFETY: the SDK wrote an error whose strings are NUL-terminated or NULL; it is released right after copying.
        let copy = unsafe {
            let e = &*error;
            let optional = |s: *const std::ffi::c_char| (!s.is_null()).then(|| isolate::string(s).into_owned());
            ConnectionError {
                time: e.time,
                address: isolate::string(e.address).into_owned(),
                category: ConnectionErrorCategory::from_sys(e.category),
                dropped: e.dropped != 0,
                exception_class_name: optional(e.exception_class_name),
                message: optional(e.message),
                count: e.count,
            }
        };
        // SAFETY: the error came from the SDK to the caller and is released once.
        crate::error_manager::report_release(
            "ConnectionError",
            call(|t| unsafe { sys::dxfg_v4_ConnectionError_release(t, error) }),
        );
        Ok(Some(copy))
    }
}

/// A message connector of an endpoint (`MessageConnector`): one per address of the address string that the endpoint
/// is connected to, such as two for `(host1:7300)(host2:7300)`; see [`DXEndpoint::get_connectors`].
///
/// It is a live view: every getter reads the connector anew, and the values are not read together as one consistent
/// set. Reading changes nothing; the statistics are not reset.
///
/// A connector holds its own handle of the SDK and nothing of its endpoint, so it never keeps the endpoint open. It
/// answers after its endpoint is closed or dropped and after a connect of the endpoint to another address replaces
/// it: such a connector is [`Disconnected`](MessageConnectorState::Disconnected) and not active, has no connections
/// and no current addresses, and keeps its name, kind, address and statistics. Cloning it is cheap, and a clone reads
/// the same connector.
///
/// ```no_run
/// use dxfeed_graal::DXEndpoint;
///
/// # fn main() -> dxfeed_graal::Result<()> {
/// let endpoint = DXEndpoint::create()?;
/// endpoint.connect("demo.dxfeed.com:7300")?;
/// for connector in endpoint.get_connectors()? {
///     let statistics = connector.get_statistics()?;
///     println!(
///         "{} {} {}: {} bytes, {} records read",
///         connector.get_name()?,
///         connector.get_address()?,
///         connector.get_state()?,
///         statistics.read_bytes,
///         statistics.read_data_records
///     );
/// }
/// # Ok(())
/// # }
/// ```
///
/// [`DXEndpoint::get_connectors`]: crate::DXEndpoint::get_connectors
#[derive(Clone)]
pub struct MessageConnector {
    handle: Arc<Handle<sys::dxfg_v4_message_connector_t>>,
}

impl fmt::Debug for MessageConnector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // No SDK call: a getter can fail, and the values change.
        f.debug_struct("MessageConnector").finish_non_exhaustive()
    }
}

impl MessageConnector {
    /// Copies the connectors of a list that the SDK returned and releases the list, also when a copy fails.
    ///
    /// # Safety
    ///
    /// `list` must be NULL or a list of live connector handles that the caller owns and releases nowhere else.
    pub(crate) unsafe fn take_list(list: *mut sys::dxfg_v4_message_connector_list) -> Result<Vec<Self>> {
        if list.is_null() {
            return Err(Error::Java(JavaException::null_result()));
        }
        // SAFETY: the caller guarantees a live list; its elements stay live until it is released below.
        let connectors = unsafe {
            let list = &*list;
            match usize::try_from(list.size) {
                Ok(size) if size > 0 && !list.elements.is_null() => std::slice::from_raw_parts(list.elements, size)
                    .iter()
                    .map(|&element| Self::clone_raw(element))
                    .collect(),
                _ => Ok(Vec::new()),
            }
        };
        // SAFETY: the list came from the SDK and is released once, after its elements are cloned.
        crate::error_manager::report_release(
            "MessageConnector",
            call(|t| unsafe { sys::dxfg_v4_message_connector_list_release(t, list) }),
        );
        connectors
    }

    /// A connector with a new handle of the connector `raw`, which stays with its owner; NULL is an error.
    ///
    /// # Safety
    ///
    /// `raw` must be NULL or a live connector handle.
    pub(crate) unsafe fn clone_raw(raw: *mut sys::dxfg_v4_message_connector_t) -> Result<Self> {
        if raw.is_null() {
            return Err(Error::Java(JavaException::null_result()));
        }
        // SAFETY: the caller guarantees a live handle.
        let handle = unsafe { Handle::clone_raw(raw) }?;
        Ok(MessageConnector {
            handle: Arc::new(handle),
        })
    }

    fn as_ptr(&self) -> *mut sys::dxfg_v4_message_connector_t {
        self.handle.as_ptr()
    }

    /// The name of the connector, as in QD's log and JMX, such as `ClientSocket-Distributor`. The connectors of
    /// different endpoints may have the same name.
    #[doc(alias = "getName")]
    pub fn get_name(&self) -> Result<String> {
        let c = self.as_ptr();
        // SAFETY: the connector is live and the out-pointer is valid for writes.
        isolate::out_string("MessageConnector", |t, name| unsafe {
            sys::dxfg_v4_MessageConnector_getName(t, c, name)
        })
    }

    /// The kind of the connector.
    pub fn get_kind(&self) -> Result<MessageConnectorKind> {
        let c = self.as_ptr();
        let mut kind = 0;
        // SAFETY: the connector is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_MessageConnector_getKind(t, c, &mut kind) })?;
        Ok(MessageConnectorKind::from_sys(kind))
    }

    /// The address the connector was configured with, such as `127.0.0.1:7300`, or `*:7400` for a server socket
    /// connector of the address `:7400`; the user and the password are masked as in QD's log.
    #[doc(alias = "getAddress")]
    pub fn get_address(&self) -> Result<String> {
        let c = self.as_ptr();
        // SAFETY: the connector is live and the out-pointer is valid for writes.
        isolate::out_string("MessageConnector", |t, address| unsafe {
            sys::dxfg_v4_MessageConnector_getAddress(t, c, address)
        })
    }

    /// The `host:port` addresses a client socket connector is connected to now, one per connection (a striped
    /// connector has several). It is empty when the connector is not connected, and always for the other kinds.
    #[doc(alias = "getCurrentAddresses")]
    pub fn get_current_addresses(&self) -> Result<Vec<String>> {
        let c = self.as_ptr();
        // SAFETY: the connector is live and the out-pointer is valid for writes.
        isolate::out_string_list("MessageConnector", |t, addresses| unsafe {
            sys::dxfg_v4_MessageConnector_getCurrentAddresses(t, c, addresses)
        })
    }

    /// The state of the connector.
    ///
    /// Returns [`Error::InvalidArgument`] for a state that this crate does not know.
    #[doc(alias = "getState")]
    pub fn get_state(&self) -> Result<MessageConnectorState> {
        let c = self.as_ptr();
        let mut state = 0;
        // SAFETY: the connector is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_MessageConnector_getState(t, c, &mut state) })?;
        MessageConnectorState::from_sys(state)
    }

    /// Whether the connector is started, so it connects or reconnects, as opposed to stopped.
    #[doc(alias = "isActive")]
    pub fn is_active(&self) -> Result<bool> {
        let c = self.as_ptr();
        let mut active = 0;
        // SAFETY: the connector is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_MessageConnector_isActive(t, c, &mut active) })?;
        Ok(active != 0)
    }

    /// The number of the open connections: those of a client socket connector (more than one for a striped
    /// connector) or those that a server socket connector accepted.
    #[doc(alias = "getConnectionCount")]
    pub fn get_connection_count(&self) -> Result<i32> {
        let c = self.as_ptr();
        let mut count = 0;
        // SAFETY: the connector is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_MessageConnector_getConnectionCount(t, c, &mut count) })?;
        Ok(count)
    }

    /// The number of the connections closed since the connector was created: it grows on each drop and on each
    /// reconnect.
    #[doc(alias = "getClosedConnectionCount")]
    pub fn get_closed_connection_count(&self) -> Result<i64> {
        let c = self.as_ptr();
        let mut count = 0;
        // SAFETY: the connector is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_MessageConnector_getClosedConnectionCount(t, c, &mut count) })?;
        Ok(count)
    }

    /// The statistics of the connector now: the bytes and the data records read and written, the open and the closed
    /// connections. Data arrives when `read_data_records` grows between two calls; `read_bytes` grows on heartbeats
    /// too.
    #[doc(alias = "retrieveCompleteEndpointStats")]
    #[doc(alias = "getStats")]
    pub fn get_statistics(&self) -> Result<MessageConnectorStatistics> {
        let c = self.as_ptr();
        let mut stats = sys::dxfg_v4_message_connector_stats_t {
            read_bytes: 0,
            written_bytes: 0,
            read_data_records: 0,
            written_data_records: 0,
            active_connection_count: 0,
            closed_connection_count: 0,
        };
        // SAFETY: the connector is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_MessageConnector_getStatistics(t, c, &mut stats) })?;
        Ok(MessageConnectorStatistics {
            read_bytes: stats.read_bytes,
            written_bytes: stats.written_bytes,
            read_data_records: stats.read_data_records,
            written_data_records: stats.written_data_records,
            active_connection_count: stats.active_connection_count,
            closed_connection_count: stats.closed_connection_count,
        })
    }

    /// The last connection error of the connector: its newest failed connect or dropped connection, or `None` when it
    /// has had no error, which does not mean that it is connected now; see [`ConnectionError`].
    ///
    /// The SDK watches the connectors of the endpoints of this crate from their creation. A connector keeps its error
    /// after it is replaced and after its endpoint is closed or dropped. Inside a
    /// [connector listener](crate::DXEndpoint::add_message_connector_listener) the error of the change that called it
    /// is already recorded.
    #[cfg(feature = "experimental")]
    pub fn get_last_error(&self) -> Result<Option<ConnectionError>> {
        let c = self.as_ptr();
        // SAFETY: the connector is live and the out-pointer is valid for writes.
        ConnectionError::last(|t, error| unsafe { sys::dxfg_v4_MessageConnector_getLastError(t, c, error) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_kind_is_other() {
        assert_eq!(MessageConnectorKind::from_sys(0), MessageConnectorKind::ClientSocket);
        assert_eq!(MessageConnectorKind::from_sys(1), MessageConnectorKind::ServerSocket);
        assert_eq!(MessageConnectorKind::from_sys(2), MessageConnectorKind::Other);
        assert_eq!(MessageConnectorKind::from_sys(3), MessageConnectorKind::Other);
        assert_eq!(MessageConnectorKind::from_sys(-1), MessageConnectorKind::Other);
    }

    #[cfg(feature = "experimental")]
    #[test]
    fn an_unknown_category_is_other() {
        let categories: Vec<_> = (-1..=8).map(ConnectionErrorCategory::from_sys).collect();
        assert_eq!(
            categories,
            [
                ConnectionErrorCategory::Other,
                ConnectionErrorCategory::Other,
                ConnectionErrorCategory::Dns,
                ConnectionErrorCategory::ConnectFailed,
                ConnectionErrorCategory::ReadTimeout,
                ConnectionErrorCategory::HeartbeatTimeout,
                ConnectionErrorCategory::ClosedByRemote,
                ConnectionErrorCategory::Tls,
                ConnectionErrorCategory::Io,
                ConnectionErrorCategory::Other,
            ]
        );
    }

    #[test]
    fn an_unknown_state_is_invalid() {
        assert_eq!(
            MessageConnectorState::from_sys(0).unwrap(),
            MessageConnectorState::Connecting
        );
        assert_eq!(
            MessageConnectorState::from_sys(1).unwrap(),
            MessageConnectorState::Connected
        );
        assert_eq!(
            MessageConnectorState::from_sys(2).unwrap(),
            MessageConnectorState::Disconnected
        );
        assert!(matches!(
            MessageConnectorState::from_sys(3),
            Err(Error::InvalidArgument(_))
        ));
        assert!(matches!(
            MessageConnectorState::from_sys(-1),
            Err(Error::InvalidArgument(_))
        ));
    }
}
