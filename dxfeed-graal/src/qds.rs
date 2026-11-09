//! The QDS tools of the SDK (Java `com.devexperts.qd.tools.Tools`): the symbol list parser and the runner of the
//! qds-tools, such as `connect`, `dump` or `help`.
//!
//! ```no_run
//! use dxfeed_graal::qds::Tools;
//!
//! # fn main() -> dxfeed_graal::Result<()> {
//! for symbol in Tools::parse_symbols("AAPL,IBM,MSFT")? {
//!     println!("{symbol}");
//! }
//! # Ok(())
//! # }
//! ```

use std::ffi::{CString, c_char};

use dxfeed_graal_sys as sys;

use crate::error::{Result, c_string};
use crate::isolate::{self, call};

/// The QDS tools of the SDK (Java `Tools`).
#[derive(Debug)]
#[non_exhaustive]
pub struct Tools;

impl Tools {
    /// Parses a symbol list, a QD symbol set filter such as `AAPL,IBM` or `*`, into its symbols (`parseSymbols`).
    ///
    /// A pattern that contains `.ipf` is the URL of an instrument profile file whose symbols make the set; reading it
    /// may need the network. The order of the symbols is not specified, and `*` stays as is. Returns
    /// [`Error::Java`] for an invalid or empty filter and [`Error::InvalidArgument`] for a NUL byte.
    ///
    /// [`Error::Java`]: crate::Error::Java
    /// [`Error::InvalidArgument`]: crate::Error::InvalidArgument
    #[doc(alias = "parseSymbols")]
    pub fn parse_symbols(symbol_list: &str) -> Result<Vec<String>> {
        let symbol_list = c_string(symbol_list, "the symbol list")?;
        // SAFETY: the string is live and the out-pointer is valid for writes.
        isolate::out_string_list("Tools", |t, list| unsafe {
            sys::dxfg_v4_Tools_parseSymbols(t, symbol_list.as_ptr(), list)
        })
    }

    /// Runs the qds-tools with command line arguments (`main`): the first argument names the tool, the others are
    /// its arguments, and `+` separates several tools run together. It blocks until the tools finish.
    ///
    /// With no arguments it prints the usage to the standard error stream and returns.
    ///
    /// **With any argument the call does not return:** as Java's `Tools.main`, it ends the whole process with
    /// `System.exit`, with the exit code 0 if the tools succeeded and 1 if they failed, as
    /// [`std::process::exit`] does. Call it only from a process that launches a tool.
    ///
    /// Returns [`Error::InvalidArgument`](crate::Error::InvalidArgument) for an argument with a NUL byte, before
    /// anything runs.
    pub fn main<S: AsRef<str>>(args: &[S]) -> Result<()> {
        let args = args
            .iter()
            .map(|arg| c_string(arg.as_ref(), "a tool argument"))
            .collect::<Result<Vec<CString>>>()?;
        let mut pointers: Vec<*const c_char> = args.iter().map(|arg| arg.as_ptr()).collect();
        let mut list = sys::dxfg_v4_string_list {
            size: pointers.len() as i32,
            elements: pointers.as_mut_ptr(),
        };
        // SAFETY: the list and its strings live until the call returns.
        call(|t| unsafe { sys::dxfg_v4_Tools_main(t, &mut list) })
    }
}
