//! Errors of the API.

use std::ffi::{CStr, CString};
use std::fmt;
use std::sync::Arc;

/// The result of a fallible call.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// An error of an SDK call.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Error {
    /// The SDK threw a Java exception.
    Java(JavaException),
    /// A GraalVM isolate function failed with this code, e.g. the isolate could not be created or a thread could not
    /// be attached.
    Graal(i32),
    /// An argument cannot be passed to the SDK, e.g. a string with a NUL byte.
    InvalidArgument(String),
    /// The SDK has been shut down, so nothing new can be created.
    ShutDown,
    /// The call is not allowed inside an event listener.
    InCallback,
    /// The wait for a result timed out, e.g. in [`Promise::wait`](crate::Promise::wait).
    Timeout,
    /// The isolate of the SDK has already been created, or its creation has failed, so its arguments cannot be set any
    /// more ([`system::set_isolate_arguments`](crate::system::set_isolate_arguments)).
    IsolateCreated,
    /// A Rust output failed, e.g. the writer given to
    /// [`InstrumentProfileWriter::write`](crate::ipf::InstrumentProfileWriter::write).
    Io(Arc<std::io::Error>),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Java(e) => e.fmt(f),
            Error::Graal(code) => write!(f, "GraalVM isolate error {code}"),
            Error::InvalidArgument(what) => write!(f, "invalid argument: {what}"),
            Error::ShutDown => f.write_str("the SDK has been shut down"),
            Error::InCallback => f.write_str("the call is not allowed inside an event listener"),
            Error::Timeout => f.write_str("the wait timed out"),
            Error::IsolateCreated => f.write_str("the isolate has already been created"),
            Error::Io(_) => f.write_str("the output failed"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Java(e) => e.source(),
            Error::Io(e) => Some(&**e),
            _ => None,
        }
    }
}

/// A copy of a Java exception thrown by the SDK.
#[derive(Debug, Clone)]
pub struct JavaException {
    class_name: String,
    message: String,
    stack_trace: String,
    cause: Option<Box<JavaException>>,
}

impl JavaException {
    pub(crate) fn new(class_name: String, message: String, stack_trace: String, cause: Option<Box<Self>>) -> Self {
        JavaException {
            class_name,
            message,
            stack_trace,
            cause,
        }
    }

    /// An exception with only a message, for SDK failures that come without a Java exception.
    pub(crate) fn from_message(message: &str) -> Self {
        Self::new(String::new(), message.to_owned(), String::new(), None)
    }

    /// The exception used when the SDK returned NULL where it promises a value.
    pub(crate) fn null_result() -> Self {
        Self::from_message("the SDK returned NULL")
    }

    /// The exception used when the SDK failed to report its own exception.
    pub(crate) fn lost() -> Self {
        Self::from_message("exception lost")
    }

    /// The fully qualified Java class name, e.g. `java.lang.IllegalArgumentException`; empty if unknown.
    pub fn get_class_name(&self) -> &str {
        &self.class_name
    }

    /// The exception message; empty if it has none.
    pub fn get_message(&self) -> &str {
        &self.message
    }

    /// The Java stack trace as `printStackTrace` prints it, causes included; empty if unknown.
    pub fn get_stack_trace(&self) -> &str {
        &self.stack_trace
    }

    /// The cause of the exception, if any.
    pub fn get_cause(&self) -> Option<&JavaException> {
        self.cause.as_deref()
    }
}

impl fmt::Display for JavaException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.class_name.is_empty(), self.message.is_empty()) {
            (false, false) => write!(f, "{}: {}", self.class_name, self.message),
            (false, true) => f.write_str(&self.class_name),
            _ => f.write_str(&self.message),
        }
    }
}

impl std::error::Error for JavaException {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.cause.as_deref().map(|c| c as _)
    }
}

/// A class of the Java exceptions that [`Promise::complete_exceptionally`](crate::Promise::complete_exceptionally)
/// and [`Promise::failed`](crate::Promise::failed) create: the fixed set that the SDK creates without reflection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum JavaExceptionClass {
    /// `java.lang.RuntimeException`.
    RuntimeException,
    /// `java.lang.IllegalArgumentException`.
    IllegalArgumentException,
    /// `java.lang.IllegalStateException`.
    IllegalStateException,
    /// `java.lang.UnsupportedOperationException`.
    UnsupportedOperationException,
    /// `java.io.IOException`.
    IOException,
    /// `java.util.concurrent.CancellationException`. It fails a promise but does not cancel it:
    /// [`Promise::is_cancelled`](crate::Promise::is_cancelled) stays `false`;
    /// [`Promise::cancel`](crate::Promise::cancel) cancels a promise.
    CancellationException,
    /// `java.util.concurrent.TimeoutException`.
    TimeoutException,
}

impl JavaExceptionClass {
    /// The fully qualified name of the class, as [`JavaException::get_class_name`] returns it.
    pub fn get_name(self) -> &'static str {
        match self {
            JavaExceptionClass::RuntimeException => "java.lang.RuntimeException",
            JavaExceptionClass::IllegalArgumentException => "java.lang.IllegalArgumentException",
            JavaExceptionClass::IllegalStateException => "java.lang.IllegalStateException",
            JavaExceptionClass::UnsupportedOperationException => "java.lang.UnsupportedOperationException",
            JavaExceptionClass::IOException => "java.io.IOException",
            JavaExceptionClass::CancellationException => "java.util.concurrent.CancellationException",
            JavaExceptionClass::TimeoutException => "java.util.concurrent.TimeoutException",
        }
    }

    /// The name as a NUL-terminated string for the SDK.
    pub(crate) fn c_name(self) -> &'static CStr {
        match self {
            JavaExceptionClass::RuntimeException => c"java.lang.RuntimeException",
            JavaExceptionClass::IllegalArgumentException => c"java.lang.IllegalArgumentException",
            JavaExceptionClass::IllegalStateException => c"java.lang.IllegalStateException",
            JavaExceptionClass::UnsupportedOperationException => c"java.lang.UnsupportedOperationException",
            JavaExceptionClass::IOException => c"java.io.IOException",
            JavaExceptionClass::CancellationException => c"java.util.concurrent.CancellationException",
            JavaExceptionClass::TimeoutException => c"java.util.concurrent.TimeoutException",
        }
    }
}

impl fmt::Display for JavaExceptionClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.get_name())
    }
}

/// The size of an SDK list of `count` elements; [`Error::InvalidArgument`] if it does not fit in `i32`.
pub(crate) fn list_size(count: usize, what: &str) -> Result<i32> {
    i32::try_from(count).map_err(|_| Error::InvalidArgument(format!("too many {what}: {count}")))
}

/// Converts `s` for the SDK; `what` names the argument in the error.
pub(crate) fn c_string(s: &str, what: &str) -> Result<CString> {
    CString::new(s).map_err(|_| Error::InvalidArgument(format!("{what} contains a NUL byte")))
}
