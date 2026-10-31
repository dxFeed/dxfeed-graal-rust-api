//! The history service: the time series events of a symbol for a period, such as candles and time and sales, over
//! HTTP.
//!
//! A [`HistoryEndpoint`] is built by a [`HistoryEndpointBuilder`] with the address of the service and the credentials:
//! a user name and a password or an authentication token. Each request blocks until the response is read.
//!
//! ```no_run
//! use dxfeed_graal::TimeFormat;
//! use dxfeed_graal::event::Candle;
//! use dxfeed_graal::history::HistoryEndpoint;
//!
//! let endpoint = HistoryEndpoint::new_builder()
//!     .with_address("https://tools-demo.dxfeed.com/candledata-preview")
//!     .with_user_name("demo")
//!     .with_password("demo")
//!     .build()?;
//! let from = TimeFormat::DEFAULT.parse("20250818-120000")?;
//! let to = TimeFormat::DEFAULT.parse("20250819-130000")?;
//! for candle in endpoint.get_time_series::<Candle>("IBM{=h}", from, to)? {
//!     println!("{candle}");
//! }
//! # Ok::<(), dxfeed_graal::Error>(())
//! ```

use std::borrow::Cow;
use std::ffi::CString;
use std::fmt;
use std::sync::Mutex;

use dxfeed_graal_sys as sys;

use crate::error::{Error, Result, c_string};
use crate::event::sdk_event::{needs_candle_symbol, take_events};
use crate::event::{EventKind, TimeSeriesEvent};
use crate::handle::Handle;
use crate::isolate::call;
use crate::symbol::{Symbol, SymbolArena};

/// The compression of the responses of the history service (`HistoryEndpoint.Compression`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Compression {
    /// No compression.
    None,
    /// ZIP, the default.
    #[default]
    Zip,
    /// GZIP.
    Gzip,
}

impl Compression {
    fn to_sys(self) -> sys::dxfg_v4_history_endpoint_compression_t {
        match self {
            Compression::None => sys::DXFG_HISTORY_ENDPOINT_COMPRESSION_NONE,
            Compression::Zip => sys::DXFG_HISTORY_ENDPOINT_COMPRESSION_ZIP,
            Compression::Gzip => sys::DXFG_HISTORY_ENDPOINT_COMPRESSION_GZIP,
        }
    }
}

/// The format of the responses of the history service (`HistoryEndpoint.Format`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Format {
    /// Text.
    Text,
    /// CSV.
    Csv,
    /// Binary, the default.
    #[default]
    Binary,
}

impl Format {
    fn to_sys(self) -> sys::dxfg_v4_history_endpoint_format_t {
        match self {
            Format::Text => sys::DXFG_HISTORY_ENDPOINT_FORMAT_TEXT,
            Format::Csv => sys::DXFG_HISTORY_ENDPOINT_FORMAT_CSV,
            Format::Binary => sys::DXFG_HISTORY_ENDPOINT_FORMAT_BINARY,
        }
    }
}

/// A client of the history service, which returns the time series events of a symbol for a period
/// (`HistoryEndpoint`).
///
/// Requests of one endpoint run one at a time, since the Java endpoint is not thread-safe; build an endpoint per
/// thread for parallel requests.
pub struct HistoryEndpoint {
    handle: Handle<sys::dxfg_v4_history_endpoint_t>,
    /// Held during a request.
    request: Mutex<()>,
}

impl HistoryEndpoint {
    /// A builder with no address and no credentials, ZIP compression and the binary format (`newBuilder`).
    #[doc(alias = "newBuilder")]
    pub fn new_builder() -> HistoryEndpointBuilder {
        HistoryEndpointBuilder::default()
    }

    /// The events of type `E` of `symbol` within the time range, in milliseconds since the Unix epoch
    /// (`getTimeSeries`). The request blocks until the response is read and parsed; a failed request, such as one to
    /// an unreachable address, returns [`Error::Java`]. The service gets the range to the second, in the local time
    /// zone.
    ///
    /// A string symbol of [`Candle`](crate::event::Candle)s is sent as a candle symbol, such as `IBM{=h}`.
    #[doc(alias = "getTimeSeries")]
    pub fn get_time_series<E: TimeSeriesEvent + EventKind>(
        &self,
        symbol: impl Into<Symbol>,
        from_time: i64,
        to_time: i64,
    ) -> Result<Vec<E>> {
        let mut arena = SymbolArena::new();
        let symbol = arena.symbol(&symbol.into(), needs_candle_symbol(E::TYPE))?;
        let e = self.handle.as_ptr();
        let _request = self.request.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        // SAFETY: the endpoint and the symbol are live; the SDK only reads the symbol. The out-pointer is valid for
        // writes.
        take_events(|t, out| unsafe {
            sys::dxfg_v4_HistoryEndpoint_getTimeSeries(t, e, E::TYPE.to_sys(), symbol, from_time, to_time, out)
        })
        .map(Option::unwrap_or_default)
    }
}

impl fmt::Debug for HistoryEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HistoryEndpoint").finish_non_exhaustive()
    }
}

/// The settings of a [`HistoryEndpoint`] (`HistoryEndpoint.Builder`). It builds any number of endpoints.
#[derive(Clone, Default)]
pub struct HistoryEndpointBuilder {
    address: Option<String>,
    user_name: Option<String>,
    password: Option<String>,
    auth_token: Option<String>,
    compression: Compression,
    format: Format,
}

impl HistoryEndpointBuilder {
    /// The URL of the service, such as `https://tools-demo.dxfeed.com/candledata-preview`; each request adds its
    /// query to it (`withAddress`).
    #[doc(alias = "withAddress")]
    pub fn with_address(mut self, address: impl Into<String>) -> Self {
        self.address = Some(address.into());
        self
    }

    /// The user name of the requests (`withUserName`).
    #[doc(alias = "withUserName")]
    pub fn with_user_name(mut self, user_name: impl Into<String>) -> Self {
        self.user_name = Some(user_name.into());
        self
    }

    /// The password of the requests (`withPassword`).
    #[doc(alias = "withPassword")]
    pub fn with_password(mut self, password: impl Into<String>) -> Self {
        self.password = Some(password.into());
        self
    }

    /// The authentication token of the requests, instead of the user name and the password (`withAuthToken`).
    #[doc(alias = "withAuthToken")]
    pub fn with_auth_token(mut self, auth_token: impl Into<String>) -> Self {
        self.auth_token = Some(auth_token.into());
        self
    }

    /// The compression of the responses (`withCompression`).
    #[doc(alias = "withCompression")]
    pub fn with_compression(mut self, compression: Compression) -> Self {
        self.compression = compression;
        self
    }

    /// The format of the responses (`withFormat`).
    #[doc(alias = "withFormat")]
    pub fn with_format(mut self, format: Format) -> Self {
        self.format = format;
        self
    }

    /// A new endpoint with the settings (`build`). Returns [`Error::InvalidArgument`] without an address or with a
    /// NUL byte in a setting.
    pub fn build(&self) -> Result<HistoryEndpoint> {
        let Some(address) = &self.address else {
            return Err(Error::InvalidArgument("the history endpoint needs an address".into()));
        };
        let address = c_string(address, "the address")?;
        let optional = |value: &Option<String>, what| value.as_deref().map(|value| c_string(value, what)).transpose();
        let user_name = optional(&self.user_name, "the user name")?;
        let password = optional(&self.password, "the password")?;
        let auth_token = optional(&self.auth_token, "the authentication token")?;

        // SAFETY: the out-pointer is valid for writes.
        let builder = Handle::create(|t, out| unsafe { sys::dxfg_v4_HistoryEndpoint_newBuilder(t, out) })?;
        let b = builder.as_ptr();
        // SAFETY: the builder and the string are live; the SDK copies the string.
        call(|t| unsafe { sys::dxfg_v4_HistoryEndpoint_Builder_withAddress(t, b, address.as_ptr()) })?;
        let strings: [(&Option<CString>, Setter); 3] = [
            (&user_name, sys::dxfg_v4_HistoryEndpoint_Builder_withUserName),
            (&password, sys::dxfg_v4_HistoryEndpoint_Builder_withPassword),
            (&auth_token, sys::dxfg_v4_HistoryEndpoint_Builder_withAuthToken),
        ];
        for (value, set) in strings {
            if let Some(value) = value {
                // SAFETY: as above.
                call(|t| unsafe { set(t, b, value.as_ptr()) })?;
            }
        }
        let compression = self.compression.to_sys();
        // SAFETY: the builder is live.
        call(|t| unsafe { sys::dxfg_v4_HistoryEndpoint_Builder_withCompression(t, b, compression) })?;
        let format = self.format.to_sys();
        // SAFETY: the builder is live.
        call(|t| unsafe { sys::dxfg_v4_HistoryEndpoint_Builder_withFormat(t, b, format) })?;
        // SAFETY: the builder is live and the out-pointer is valid for writes.
        let handle = Handle::create(|t, out| unsafe { sys::dxfg_v4_HistoryEndpoint_Builder_build(t, b, out) })?;
        Ok(HistoryEndpoint {
            handle,
            request: Mutex::new(()),
        })
    }
}

/// A string setter of the SDK builder.
type Setter = unsafe extern "C" fn(
    *mut sys::graal_isolatethread_t,
    *mut sys::dxfg_v4_history_endpoint_builder_t,
    *const std::ffi::c_char,
) -> i32;

impl fmt::Debug for HistoryEndpointBuilder {
    /// The settings without the password, the token and the user information of the address.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HistoryEndpointBuilder")
            .field("address", &self.address.as_deref().map(without_user_info))
            .field("user_name", &self.user_name)
            .field("compression", &self.compression)
            .field("format", &self.format)
            .finish_non_exhaustive()
    }
}

/// The address without the user information of a URL, such as the password of `https://user:password@host/path`.
fn without_user_info(address: &str) -> Cow<'_, str> {
    let Some(scheme_end) = address.find("://").map(|i| i + 3) else {
        return Cow::Borrowed(address);
    };
    let rest = &address[scheme_end..];
    let authority = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
    match authority.rfind('@') {
        Some(at) => Cow::Owned(format!("{}***@{}", &address[..scheme_end], &rest[at + 1..])),
        None => Cow::Borrowed(address),
    }
}
