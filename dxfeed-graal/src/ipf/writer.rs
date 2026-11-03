//! The writer of instrument profiles to files and memory.

use std::io::Write;
use std::ptr;
use std::sync::Arc;

use dxfeed_graal_sys as sys;

use crate::error::{Error, JavaException, Result, c_string};
use crate::ipf::InstrumentProfile;
use crate::ipf::profile::SysProfileList;
use crate::isolate::call;

/// Writes instrument profiles in the Instrument Profile Format to files and memory (`InstrumentProfileWriter`).
///
/// The name of the file selects the compression, as in Java, in any case of the letters: a name that ends with `.zip`
/// gives a zip archive with one entry, named as the file without its directory (after the last `/` or `\`) and
/// `.zip`, whose name selects the format of the entry in turn; a name that ends with `.gz` gives gzip data; any other
/// name gives plain text. For each type of the profiles a header lists the fields that have values, the custom fields
/// included, and the data ends with the `##COMPLETE` tag. The lines end with `\r\n` on every platform, as the CSV
/// that Java writes.
///
/// [`InstrumentProfileReader`](crate::ipf::InstrumentProfileReader) reads the data back, except gzip data in a zip
/// entry, such as that of `profiles.ipf.gz.zip`: as in Java, the reader takes the entry for plain text.
///
/// ```no_run
/// use dxfeed_graal::ipf::{InstrumentProfileReader, InstrumentProfileWriter};
///
/// let profiles = InstrumentProfileReader::new()?.read_from_file("https://tools.dxfeed.com/ipf?TYPE=STOCK")?;
/// InstrumentProfileWriter::new().write_to_file("stocks.ipf.zip", &profiles)?;
/// # Ok::<(), dxfeed_graal::Error>(())
/// ```
#[derive(Debug, Default, Clone, Copy)]
pub struct InstrumentProfileWriter {
    _private: (),
}

impl InstrumentProfileWriter {
    /// Creates a writer.
    pub fn new() -> Self {
        InstrumentProfileWriter::default()
    }

    /// Writes `profiles` to `file`, compressed as its name tells; an existing file is overwritten (`writeToFile`).
    ///
    /// An I/O error, such as a missing directory, returns [`Error::Java`] of `java.io.IOException` or its subclass
    /// `java.io.FileNotFoundException` for a file that cannot be opened; a file that was opened may be left
    /// incomplete.
    #[doc(alias = "writeToFile")]
    pub fn write_to_file(&self, file: &str, profiles: &[InstrumentProfile]) -> Result<()> {
        let file = c_string(file, "the file")?;
        let profiles = SysProfileList::new(profiles)?;
        // SAFETY: the file name and the list with its structures are live; the SDK only reads them.
        call(|t| unsafe { sys::dxfg_v4_InstrumentProfileWriter_writeToFile(t, file.as_ptr(), &profiles.list) })
    }

    /// Writes `profiles` to `out` as [`write_to_bytes`](Self::write_to_bytes) gives them for `name`
    /// (`write(OutputStream, String, List)`); `out` is not flushed. Java's `write(OutputStream, List)` is this method
    /// with a name that ends with neither `.zip` nor `.gz`.
    ///
    /// An error of `out` returns [`Error::Io`]; the data is made before anything is written to `out`.
    pub fn write<W: Write + ?Sized>(&self, out: &mut W, name: &str, profiles: &[InstrumentProfile]) -> Result<()> {
        let bytes = self.write_to_bytes(name, profiles)?;
        out.write_all(&bytes).map_err(|e| Error::Io(Arc::new(e)))
    }

    /// The bytes of a file named `name` that [`write_to_file`](Self::write_to_file) writes with `profiles`; nothing is
    /// written to a file.
    /// [`InstrumentProfileReader::read_from_bytes`](crate::ipf::InstrumentProfileReader::read_from_bytes) reads them
    /// back.
    ///
    /// Data of more than 2147483639 bytes returns [`Error::Java`] of `java.lang.IllegalStateException`.
    pub fn write_to_bytes(&self, name: &str, profiles: &[InstrumentProfile]) -> Result<Vec<u8>> {
        let name = c_string(name, "the name")?;
        let profiles = SysProfileList::new(profiles)?;
        let mut bytes = ptr::null_mut();
        // SAFETY: the name and the list with its structures are live, the SDK only reads them; the out-pointer is
        // valid for writes.
        call(|t| unsafe {
            sys::dxfg_v4_InstrumentProfileWriter_writeToBytes(t, name.as_ptr(), &profiles.list, &mut bytes)
        })?;
        if bytes.is_null() {
            return Err(Error::Java(JavaException::null_result()));
        }
        // SAFETY: the SDK filled the list, which has `size` bytes, or none and maybe a NULL array.
        let copy = unsafe {
            let list = &*bytes;
            if list.elements.is_null() || list.size <= 0 {
                Vec::new()
            } else {
                std::slice::from_raw_parts(list.elements.cast::<u8>(), list.size as usize).to_vec()
            }
        };
        // SAFETY: the list is ours to release once; nothing uses it afterwards.
        crate::error_manager::report_release(
            "InstrumentProfileWriter",
            call(|t| unsafe { sys::dxfg_v4_byte_list_release(t, bytes) }),
        );
        Ok(copy)
    }
}
