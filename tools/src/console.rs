//! The output of the tools and the width of the console the help screens wrap to.

use std::io::Write;

/// The width used when the output is not a console and `COLUMNS` is not set.
const DEFAULT_WIDTH: usize = 80;

/// The number of columns of the console that the standard output goes to; else the `COLUMNS` environment variable;
/// else 80.
pub fn width() -> usize {
    platform::width()
        .filter(|&width| width > 0)
        .or_else(|| {
            std::env::var("COLUMNS")
                .ok()?
                .trim()
                .parse()
                .ok()
                .filter(|&width| width > 0)
        })
        .unwrap_or(DEFAULT_WIDTH)
}

/// Writes `text` to the standard output; a closed output, such as a pipe whose reader is gone, only ends the writing.
pub fn print(text: &str) {
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(text.as_bytes()).and_then(|()| out.flush());
}

/// Writes `text` and a line break to the standard output, as [`print`] does.
pub fn println(text: &str) {
    print(&format!("{text}\n"));
}

#[cfg(windows)]
mod platform {
    use std::ffi::c_void;

    #[repr(C)]
    struct Coord {
        x: i16,
        y: i16,
    }

    #[repr(C)]
    struct SmallRect {
        left: i16,
        top: i16,
        right: i16,
        bottom: i16,
    }

    #[repr(C)]
    struct ConsoleScreenBufferInfo {
        size: Coord,
        cursor_position: Coord,
        attributes: u16,
        window: SmallRect,
        maximum_window_size: Coord,
    }

    /// `STD_OUTPUT_HANDLE`, `(DWORD)-11`.
    const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetStdHandle(std_handle: u32) -> *mut c_void;
        fn GetConsoleScreenBufferInfo(console: *mut c_void, info: *mut ConsoleScreenBufferInfo) -> i32;
    }

    pub fn width() -> Option<usize> {
        // SAFETY: GetStdHandle takes no pointers.
        let handle = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
        if handle.is_null() || handle as isize == -1 {
            return None;
        }
        let mut info = ConsoleScreenBufferInfo {
            size: Coord { x: 0, y: 0 },
            cursor_position: Coord { x: 0, y: 0 },
            attributes: 0,
            window: SmallRect {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            },
            maximum_window_size: Coord { x: 0, y: 0 },
        };
        // SAFETY: the handle is a standard handle and the out-pointer is valid for writes; a handle that is not a
        // console makes the call fail.
        if unsafe { GetConsoleScreenBufferInfo(handle, &mut info) } == 0 {
            return None;
        }
        usize::try_from(i32::from(info.window.right) - i32::from(info.window.left) + 1).ok()
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod platform {
    use std::ffi::{c_int, c_ulong};

    #[repr(C)]
    struct Winsize {
        rows: u16,
        columns: u16,
        x_pixels: u16,
        y_pixels: u16,
    }

    #[cfg(target_os = "linux")]
    const TIOCGWINSZ: c_ulong = 0x5413;
    #[cfg(target_os = "macos")]
    const TIOCGWINSZ: c_ulong = 0x4008_7468;

    unsafe extern "C" {
        fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
    }

    pub fn width() -> Option<usize> {
        let mut size = Winsize {
            rows: 0,
            columns: 0,
            x_pixels: 0,
            y_pixels: 0,
        };
        // SAFETY: TIOCGWINSZ writes a winsize to the valid out-pointer; a descriptor that is not a terminal makes the
        // call fail.
        if unsafe { ioctl(1, TIOCGWINSZ, &mut size as *mut Winsize) } != 0 {
            return None;
        }
        Some(usize::from(size.columns))
    }
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
mod platform {
    pub fn width() -> Option<usize> {
        None
    }
}
