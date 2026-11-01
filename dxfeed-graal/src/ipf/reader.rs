//! The reader of instrument profiles from files and URLs.

use std::ffi::CString;
use std::ptr;

use dxfeed_graal_sys as sys;

use crate::AuthToken;
use crate::error::{Error, JavaException, Result, c_string};
use crate::handle::Handle;
use crate::ipf::InstrumentProfile;
use crate::ipf::profile::take_string;
use crate::isolate::call;

/// Reads instrument profiles in the Instrument Profile Format from files and URLs (`InstrumentProfileReader`).
///
/// The address is a file path, a URL such as `https://tools.dxfeed.com/ipf` or `<host>:<port>`; zip and gzip data
/// is unpacked. Credentials can be a part of the URL, `https://user:password@host/path`, or are given to
/// [`read_from_file_with_credentials`](Self::read_from_file_with_credentials) or
/// [`read_from_file_with_token`](Self::read_from_file_with_token). [`read_from_bytes`](Self::read_from_bytes) reads
/// data that is already in memory.
///
/// ```no_run
/// use dxfeed_graal::ipf::InstrumentProfileReader;
///
/// let profiles = InstrumentProfileReader::new()?.read_from_file("profiles.ipf.zip")?;
/// for profile in &profiles {
///     println!("{profile}");
/// }
/// # Ok::<(), dxfeed_graal::Error>(())
/// ```
pub struct InstrumentProfileReader {
    handle: Handle<sys::dxfg_v4_instrument_profile_reader_t>,
}

impl InstrumentProfileReader {
    /// Creates a reader.
    pub fn new() -> Result<Self> {
        // SAFETY: the out-pointer is valid for writes.
        let handle = Handle::create(|t, out| unsafe { sys::dxfg_v4_InstrumentProfileReader_new(t, out) })?;
        Ok(InstrumentProfileReader { handle })
    }

    /// The URL that an address stands for: `<host>:<port>` gives `http://<host>:<port>/ipf/all.ipf.gz`, file paths
    /// and URLs stay as they are (`resolveSourceURL`).
    #[doc(alias = "resolveSourceURL")]
    pub fn resolve_source_url(address: &str) -> Result<String> {
        let address = c_string(address, "the address")?;
        let mut url = ptr::null();
        // SAFETY: the address is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_InstrumentProfileReader_resolveSourceURL(t, address.as_ptr(), &mut url) })?;
        // SAFETY: the SDK wrote a NUL-terminated string or NULL, which is ours to release.
        Ok(unsafe { take_string(url) })
    }

    /// The last modification time of the last file read, in milliseconds since the Unix epoch, or `0` if it is not
    /// known (`getLastModified`).
    #[doc(alias = "getLastModified")]
    pub fn get_last_modified(&self) -> Result<i64> {
        let mut time = 0;
        let r = self.handle.as_ptr();
        // SAFETY: the reader is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_InstrumentProfileReader_getLastModified(t, r, &mut time) })?;
        Ok(time)
    }

    /// Whether the last file read was complete: its data ended with the end-of-file mark of the format
    /// (`wasComplete`).
    #[doc(alias = "wasComplete")]
    pub fn was_complete(&self) -> Result<bool> {
        let mut complete = 0;
        let r = self.handle.as_ptr();
        // SAFETY: the reader is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_InstrumentProfileReader_wasComplete(t, r, &mut complete) })?;
        Ok(complete != 0)
    }

    /// Reads the profiles from an address (`readFromFile`).
    ///
    /// Returns an error if the data does not conform to the format or cannot be read.
    #[doc(alias = "readFromFile")]
    pub fn read_from_file(&self, address: &str) -> Result<Vec<InstrumentProfile>> {
        let address = c_string(address, "the address")?;
        let r = self.handle.as_ptr();
        self.read(|t, out| {
            // SAFETY: the reader and the address are live; the out-pointer is valid for writes.
            unsafe { sys::dxfg_v4_InstrumentProfileReader_readFromFile(t, r, address.as_ptr(), out) }
        })
    }

    /// Reads the profiles from an address with the user and the password of basic authentication, which take
    /// precedence over those in the URL (`readFromFile`).
    #[doc(alias = "readFromFile")]
    pub fn read_from_file_with_credentials(
        &self,
        address: &str,
        user: &str,
        password: &str,
    ) -> Result<Vec<InstrumentProfile>> {
        let address = c_string(address, "the address")?;
        let user: CString = c_string(user, "the user")?;
        let password = c_string(password, "the password")?;
        let r = self.handle.as_ptr();
        self.read(|t, out| {
            // SAFETY: the reader and the strings are live; the out-pointer is valid for writes.
            unsafe {
                sys::dxfg_v4_InstrumentProfileReader_readFromFileWithUserAndPassword(
                    t,
                    r,
                    address.as_ptr(),
                    user.as_ptr(),
                    password.as_ptr(),
                    out,
                )
            }
        })
    }

    /// Reads the profiles from an address with an authorization token, which takes precedence over the credentials
    /// in the URL (`readFromFile(String, AuthToken)`).
    #[doc(alias = "readFromFile")]
    pub fn read_from_file_with_token(&self, address: &str, token: &AuthToken) -> Result<Vec<InstrumentProfile>> {
        let address = c_string(address, "the address")?;
        let token = token.to_sdk()?;
        let (r, token) = (self.handle.as_ptr(), token.as_ptr());
        self.read(|t, out| {
            // SAFETY: the reader, the address and the token are live; the out-pointer is valid for writes.
            unsafe { sys::dxfg_v4_InstrumentProfileReader_readFromFileWithToken(t, r, address.as_ptr(), token, out) }
        })
    }

    /// Reads the profiles from data in memory, as a file at `address` would give them: zip and gzip data is
    /// unpacked; `address` only names the data in the warning about incomplete data (`read(InputStream, String)`).
    ///
    /// It updates [`was_complete`](Self::was_complete), not [`get_last_modified`](Self::get_last_modified).
    #[doc(alias = "read")]
    pub fn read_from_bytes(&self, bytes: &[u8], address: &str) -> Result<Vec<InstrumentProfile>> {
        let address = c_string(address, "the address")?;
        let size = i32::try_from(bytes.len())
            .map_err(|_| Error::InvalidArgument(format!("{} bytes do not fit in a Java array", bytes.len())))?;
        let stream = Handle::create(|t, out| {
            // SAFETY: `bytes` has `size` bytes, which the SDK copies; the out-pointer is valid for writes.
            unsafe { sys::dxfg_v4_ByteArrayInputStream_new(t, bytes.as_ptr().cast(), size, out) }
        })?;
        let (r, stream) = (self.handle.as_ptr(), stream.as_ptr());
        self.read(|t, out| {
            // SAFETY: the reader, the stream and the address are live; the out-pointer is valid for writes.
            unsafe { sys::dxfg_v4_InstrumentProfileReader_readWithAddress(t, r, stream, address.as_ptr(), out) }
        })
    }

    fn read(
        &self,
        f: impl FnOnce(crate::isolate::Thread, *mut *mut sys::dxfg_v4_instrument_profile_data_list) -> i32,
    ) -> Result<Vec<InstrumentProfile>> {
        let mut list = ptr::null_mut();
        call(|t| f(t, &mut list))?;
        if list.is_null() {
            return Err(Error::Java(JavaException::null_result()));
        }
        // SAFETY: the SDK filled the list, which is released once after the copy.
        let profiles = unsafe { profiles(list) };
        // SAFETY: as above; nothing uses it afterwards.
        crate::error_manager::report_release(
            "InstrumentProfileReader",
            call(|t| unsafe { sys::dxfg_v4_instrument_profile_data_list_release(t, list) }),
        );
        profiles
    }
}

/// Copies the profiles of an SDK list.
///
/// # Safety
///
/// `list` must point to a live list.
unsafe fn profiles(list: *const sys::dxfg_v4_instrument_profile_data_list) -> Result<Vec<InstrumentProfile>> {
    // SAFETY: the caller guarantees the list is live.
    let list = unsafe { &*list };
    if list.size == 0 {
        return Ok(Vec::new());
    }
    // SAFETY: the list has `size` elements, each a live structure.
    unsafe { std::slice::from_raw_parts(list.elements, list.size as usize) }
        .iter()
        .map(|&data| unsafe { InstrumentProfile::from_sys(data) })
        .collect()
}
