//! Authorization tokens (`AuthToken`).

use std::ffi::c_char;
use std::fmt::{self, Display, Formatter};
use std::hash::{Hash, Hasher};
use std::str::FromStr;

use dxfeed_graal_sys as sys;

use crate::error::{Result, c_string};
use crate::handle::Handle;
use crate::isolate::{Thread, out_optional_string, out_string};

/// An authorization token: a scheme and a value, as the HTTP `Authorization` header carries them (`AuthToken`).
///
/// The SDK makes the tokens, so they follow the rules of Java: the value of a [`BASIC_SCHEME`](Self::BASIC_SCHEME)
/// token is the Base64 of `user:password`, and a [`BEARER_SCHEME`](Self::BEARER_SCHEME) token carries an access
/// token. Two tokens are equal when their [`get_http_authorization`](Self::get_http_authorization) strings are, and
/// [`Display`] writes that string, as Java's `toString` does. [`Debug`](fmt::Debug) shows the scheme only.
///
/// ```
/// # fn main() -> dxfeed_graal::Result<()> {
/// use dxfeed_graal::AuthToken;
///
/// let token = AuthToken::create_basic_token("user", "secret")?;
/// assert_eq!(token.get_http_authorization(), "Basic dXNlcjpzZWNyZXQ=");
/// assert_eq!(token.get_user(), Some("user"));
/// assert_eq!("Basic dXNlcjpzZWNyZXQ=".parse::<AuthToken>()?, token);
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct AuthToken {
    http_authorization: String,
    scheme: String,
    value: String,
    user: Option<String>,
    password: Option<String>,
}

impl AuthToken {
    /// The scheme of a token of a user name and a password (RFC 2617), `Basic` (`AuthToken.BASIC_SCHEME`).
    pub const BASIC_SCHEME: &'static str = "Basic";

    /// The scheme of a token of an access token (RFC 6750), `Bearer` (`AuthToken.BEARER_SCHEME`).
    pub const BEARER_SCHEME: &'static str = "Bearer";

    /// Parses the scheme and the value separated by a space, such as `Bearer <token>` or
    /// `Basic <Base64 of user:password>` (`AuthToken.valueOf`).
    ///
    /// Fails with [`Error::Java`](crate::Error::Java) without a space, or for a `Basic` value that is not the Base64
    /// of `user:password`, and with [`Error::InvalidArgument`](crate::Error::InvalidArgument) for a NUL byte.
    #[doc(alias = "valueOf")]
    pub fn value_of(string: &str) -> Result<AuthToken> {
        let string = c_string(string, "the token")?;
        // SAFETY: the string is NUL-terminated and the out-pointer is valid for writes.
        Self::create(|t, out| unsafe { sys::dxfg_v4_AuthToken_valueOf(t, string.as_ptr(), out) })
    }

    /// A [`BASIC_SCHEME`](Self::BASIC_SCHEME) token of `user` and `password`, which can be empty
    /// (`AuthToken.createBasicToken(String, String)`).
    ///
    /// Fails with [`Error::InvalidArgument`](crate::Error::InvalidArgument) for a NUL byte.
    #[doc(alias = "createBasicToken")]
    pub fn create_basic_token(user: &str, password: &str) -> Result<AuthToken> {
        let (user, password) = (c_string(user, "the user name")?, c_string(password, "the password")?);
        // SAFETY: the strings are NUL-terminated and the out-pointer is valid for writes.
        Self::create(|t, out| unsafe {
            sys::dxfg_v4_AuthToken_createBasicTokenWithUserAndPassword(t, user.as_ptr(), password.as_ptr(), out)
        })
    }

    /// A [`BASIC_SCHEME`](Self::BASIC_SCHEME) token of the string `user:password`, split at its first colon
    /// (`AuthToken.createBasicToken(String)`).
    ///
    /// Fails with [`Error::Java`](crate::Error::Java) without a colon and with
    /// [`Error::InvalidArgument`](crate::Error::InvalidArgument) for a NUL byte.
    #[doc(alias = "createBasicToken")]
    pub fn create_basic_token_from(user_password: &str) -> Result<AuthToken> {
        let user_password = c_string(user_password, "the user name and the password")?;
        // SAFETY: the string is NUL-terminated and the out-pointer is valid for writes.
        Self::create(|t, out| unsafe { sys::dxfg_v4_AuthToken_createBasicToken(t, user_password.as_ptr(), out) })
    }

    /// A [`BASIC_SCHEME`](Self::BASIC_SCHEME) token of `user` and `password`, where `None` stands for an empty
    /// string; `None` when both are empty (`AuthToken.createBasicTokenOrNull`).
    #[doc(alias = "createBasicTokenOrNull")]
    pub fn create_basic_token_or_none(user: Option<&str>, password: Option<&str>) -> Result<Option<AuthToken>> {
        let (user, password) = (user.unwrap_or_default(), password.unwrap_or_default());
        if user.is_empty() && password.is_empty() {
            return Ok(None);
        }
        Self::create_basic_token(user, password).map(Some)
    }

    /// A [`BEARER_SCHEME`](Self::BEARER_SCHEME) token of the access token `token` (`AuthToken.createBearerToken`).
    ///
    /// Fails with [`Error::Java`](crate::Error::Java) for an empty token and with
    /// [`Error::InvalidArgument`](crate::Error::InvalidArgument) for a NUL byte.
    #[doc(alias = "createBearerToken")]
    pub fn create_bearer_token(token: &str) -> Result<AuthToken> {
        let token = c_string(token, "the token")?;
        // SAFETY: the string is NUL-terminated and the out-pointer is valid for writes.
        Self::create(|t, out| unsafe { sys::dxfg_v4_AuthToken_createBearerToken(t, token.as_ptr(), out) })
    }

    /// A [`BEARER_SCHEME`](Self::BEARER_SCHEME) token of the access token `token`; `None` when it is `None` or empty
    /// (`AuthToken.createBearerTokenOrNull`).
    #[doc(alias = "createBearerTokenOrNull")]
    pub fn create_bearer_token_or_none(token: Option<&str>) -> Result<Option<AuthToken>> {
        match token {
            Some(token) if !token.is_empty() => Self::create_bearer_token(token).map(Some),
            _ => Ok(None),
        }
    }

    /// A token of a custom `scheme` and `value` (`AuthToken.createCustomToken`).
    ///
    /// Fails with [`Error::Java`](crate::Error::Java) for an empty scheme or value and with
    /// [`Error::InvalidArgument`](crate::Error::InvalidArgument) for a NUL byte.
    #[doc(alias = "createCustomToken")]
    pub fn create_custom_token(scheme: &str, value: &str) -> Result<AuthToken> {
        let (scheme, value) = (c_string(scheme, "the scheme")?, c_string(value, "the value")?);
        // SAFETY: the strings are NUL-terminated and the out-pointer is valid for writes.
        Self::create(|t, out| unsafe {
            sys::dxfg_v4_AuthToken_createCustomToken(t, scheme.as_ptr(), value.as_ptr(), out)
        })
    }

    /// The value of the HTTP `Authorization` header: the scheme and the value separated by a space
    /// (`getHttpAuthorization`).
    #[doc(alias = "getHttpAuthorization")]
    pub fn get_http_authorization(&self) -> &str {
        &self.http_authorization
    }

    /// The user name of a [`BASIC_SCHEME`](Self::BASIC_SCHEME) token made from a user name and a password or parsed
    /// by [`value_of`](Self::value_of); `None` for the other tokens (`getUser`).
    #[doc(alias = "getUser")]
    pub fn get_user(&self) -> Option<&str> {
        self.user.as_deref()
    }

    /// The password of a token that has a [`get_user`](Self::get_user); `None` for the other tokens
    /// (`getPassword`).
    #[doc(alias = "getPassword")]
    pub fn get_password(&self) -> Option<&str> {
        self.password.as_deref()
    }

    /// The scheme, such as [`BASIC_SCHEME`](Self::BASIC_SCHEME) or [`BEARER_SCHEME`](Self::BEARER_SCHEME)
    /// (`getScheme`).
    #[doc(alias = "getScheme")]
    pub fn get_scheme(&self) -> &str {
        &self.scheme
    }

    /// The value: the access token of a [`BEARER_SCHEME`](Self::BEARER_SCHEME) token, the Base64 of `user:password`
    /// of a [`BASIC_SCHEME`](Self::BASIC_SCHEME) one (`getValue`).
    #[doc(alias = "getValue")]
    pub fn get_value(&self) -> &str {
        &self.value
    }

    /// Makes a token with `f` and copies its strings; the SDK token is released.
    fn create(f: impl FnOnce(Thread, *mut *mut sys::dxfg_v4_auth_token_t) -> i32) -> Result<AuthToken> {
        let handle = Handle::create(f)?;
        let token = handle.as_ptr();
        let string = |get: Getter| {
            // SAFETY: the token is live while `handle` is, and the out-pointer is valid for writes.
            out_string("AuthToken", |t, out| unsafe { get(t, token, out) })
        };
        let optional = |get: Getter| {
            // SAFETY: as above.
            out_optional_string("AuthToken", |t, out| unsafe { get(t, token, out) })
        };
        Ok(AuthToken {
            http_authorization: string(sys::dxfg_v4_AuthToken_getHttpAuthorization)?,
            scheme: string(sys::dxfg_v4_AuthToken_getScheme)?,
            value: string(sys::dxfg_v4_AuthToken_getValue)?,
            user: optional(sys::dxfg_v4_AuthToken_getUser)?,
            password: optional(sys::dxfg_v4_AuthToken_getPassword)?,
        })
    }
}

/// A string getter of an SDK token.
type Getter =
    unsafe extern "C" fn(*mut sys::graal_isolatethread_t, *mut sys::dxfg_v4_auth_token_t, *mut *const c_char) -> i32;

impl PartialEq for AuthToken {
    fn eq(&self, other: &Self) -> bool {
        self.http_authorization == other.http_authorization
    }
}

impl Eq for AuthToken {}

impl Hash for AuthToken {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.http_authorization.hash(state);
    }
}

impl Display for AuthToken {
    /// Writes [`get_http_authorization`](Self::get_http_authorization), as Java's `toString` does.
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(&self.http_authorization)
    }
}

impl fmt::Debug for AuthToken {
    /// Shows the scheme only: the rest is a secret.
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthToken")
            .field("scheme", &self.scheme)
            .finish_non_exhaustive()
    }
}

impl FromStr for AuthToken {
    type Err = crate::Error;

    /// Parses the token as [`value_of`](AuthToken::value_of) does.
    fn from_str(s: &str) -> Result<Self> {
        Self::value_of(s)
    }
}
