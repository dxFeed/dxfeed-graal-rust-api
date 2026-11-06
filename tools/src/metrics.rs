//! The CPU time and the memory of the process, the platform line and the timestamps of the performance tools.

use std::num::NonZero;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dxfeed_graal::TimeZone;

/// The CPU time the process has used, in user and kernel mode; `None` where it is not known.
pub fn cpu_time() -> Option<Duration> {
    platform::cpu_time()
}

/// The resident memory (the working set) of the process, in bytes; `None` where it is not known.
pub fn memory() -> Option<u64> {
    platform::memory()
}

/// The number of logical cores.
pub fn cores() -> usize {
    std::thread::available_parallelism().map_or(1, NonZero::get)
}

/// The platform as the C++ tools name it, such as `Windows x86_64(16 cores)`.
pub fn platform_info() -> String {
    let cores = cores();
    format!(
        "{} {}({cores} core{})",
        os_name(),
        architecture(),
        if cores > 1 { "s" } else { "" }
    )
}

fn os_name() -> &'static str {
    match std::env::consts::OS {
        "windows" => "Windows",
        "android" => "Android",
        "linux" => "Linux",
        "solaris" | "illumos" => "Solaris",
        "macos" => "MacOS",
        "ios" | "tvos" | "watchos" | "visionos" => "iOS|tvOS|watchOS",
        "freebsd" => "FreeBSD",
        "dragonfly" => "DragonFly",
        "netbsd" => "NetBSD",
        "openbsd" => "OpenBSD",
        "aix" => "AIX",
        "haiku" => "Haiku",
        "nto" => "QNX",
        _ if cfg!(unix) => "UNIX-like",
        _ => "Unknown",
    }
}

fn architecture() -> &'static str {
    match std::env::consts::ARCH {
        "x86" => "x86_32",
        "mips" | "mips64" | "mips32r6" | "mips64r6" => "mips",
        "powerpc64" => "ppc64",
        "powerpc" => "powerpc",
        "sparc" | "sparc64" => "spark",
        "s390x" => "systemz",
        other => other,
    }
}

/// The local time `millis` (milliseconds since the Unix epoch) as `2026-11-06 18:06:50.123`, as the C++ tools write
/// the timestamps of their CSV lines; `0` for a time not after the epoch.
pub fn local_timestamp(millis: i64) -> String {
    if millis <= 0 {
        return "0".to_owned();
    }
    let offset = TimeZone::get_default()
        .and_then(|zone| zone.get_offset(millis))
        .unwrap_or(0);
    let local = millis + i64::from(offset);
    let (days, millis_of_day) = (local.div_euclid(86_400_000), local.rem_euclid(86_400_000));
    let (year, month, day) = civil_date(days);
    let seconds = millis_of_day / 1000;
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}.{:03}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60,
        millis_of_day % 1000
    )
}

/// The date of `days` since 1970-01-01 in the proleptic Gregorian calendar.
fn civil_date(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days.rem_euclid(146_097);
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// The current time in milliseconds since the Unix epoch.
pub fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| i64::try_from(since.as_millis()).unwrap_or(i64::MAX))
}

/// `duration` as `HH:MM:SS.mmm`, as the C++ tools print a running time, but the hours not wrapping at a day.
pub fn hms(duration: Duration) -> String {
    let seconds = duration.as_secs();
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60,
        duration.subsec_millis()
    )
}

#[cfg(windows)]
mod platform {
    use std::ffi::c_void;
    use std::time::Duration;

    #[repr(C)]
    #[derive(Default)]
    struct FileTime {
        low: u32,
        high: u32,
    }

    impl FileTime {
        /// The time in units of 100 nanoseconds.
        fn ticks(&self) -> u64 {
            (u64::from(self.high) << 32) | u64::from(self.low)
        }
    }

    #[repr(C)]
    #[derive(Default)]
    struct ProcessMemoryCounters {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
        fn GetProcessTimes(
            process: *mut c_void,
            creation: *mut FileTime,
            exit: *mut FileTime,
            kernel: *mut FileTime,
            user: *mut FileTime,
        ) -> i32;
        fn K32GetProcessMemoryInfo(process: *mut c_void, counters: *mut ProcessMemoryCounters, size: u32) -> i32;
    }

    pub fn cpu_time() -> Option<Duration> {
        let (mut creation, mut exit, mut kernel, mut user) = Default::default();
        // SAFETY: the pseudo handle of the current process needs no closing, and the out-pointers are valid for
        // writes.
        let ok = unsafe { GetProcessTimes(GetCurrentProcess(), &mut creation, &mut exit, &mut kernel, &mut user) };
        (ok != 0).then(|| Duration::from_nanos((kernel.ticks() + user.ticks()) * 100))
    }

    pub fn memory() -> Option<u64> {
        let mut counters = ProcessMemoryCounters::default();
        let size = size_of::<ProcessMemoryCounters>() as u32;
        counters.cb = size;
        // SAFETY: the pseudo handle of the current process needs no closing, and the out-pointer is valid for writes
        // of `size` bytes.
        let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, size) };
        (ok != 0).then_some(counters.working_set_size as u64)
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod platform {
    use std::ffi::{c_int, c_long};
    use std::time::Duration;

    /// `timeval`: two `long` fields on Linux, a 64-bit `time_t` and a 32-bit `suseconds_t` on macOS.
    #[repr(C)]
    struct Timeval {
        #[cfg(target_os = "linux")]
        seconds: c_long,
        #[cfg(target_os = "linux")]
        microseconds: c_long,
        #[cfg(target_os = "macos")]
        seconds: i64,
        #[cfg(target_os = "macos")]
        microseconds: i32,
    }

    #[repr(C)]
    struct Rusage {
        user: Timeval,
        system: Timeval,
        rest: [c_long; 14],
    }

    const RUSAGE_SELF: c_int = 0;

    unsafe extern "C" {
        fn getrusage(who: c_int, usage: *mut Rusage) -> c_int;
    }

    fn duration(time: &Timeval) -> Duration {
        Duration::from_secs(u64::try_from(time.seconds).unwrap_or(0))
            + Duration::from_micros(u64::try_from(time.microseconds).unwrap_or(0))
    }

    pub fn cpu_time() -> Option<Duration> {
        let mut usage = std::mem::MaybeUninit::<Rusage>::zeroed();
        // SAFETY: getrusage writes a rusage to the valid out-pointer.
        if unsafe { getrusage(RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
            return None;
        }
        // SAFETY: the call succeeded and filled the struct, which was zeroed before.
        let usage = unsafe { usage.assume_init() };
        Some(duration(&usage.user) + duration(&usage.system))
    }

    #[cfg(target_os = "linux")]
    pub fn memory() -> Option<u64> {
        const SC_PAGESIZE: c_int = 30;
        unsafe extern "C" {
            fn sysconf(name: c_int) -> c_long;
        }
        let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
        let resident_pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
        // SAFETY: sysconf takes no pointers.
        let page_size = u64::try_from(unsafe { sysconf(SC_PAGESIZE) }).ok()?;
        Some(resident_pages * page_size)
    }

    #[cfg(target_os = "macos")]
    pub fn memory() -> Option<u64> {
        /// `mach_task_basic_info`, packed to 4 bytes as the system headers pack it.
        #[repr(C, packed(4))]
        #[derive(Default)]
        struct MachTaskBasicInfo {
            virtual_size: u64,
            resident_size: u64,
            resident_size_max: u64,
            user_time: [i32; 2],
            system_time: [i32; 2],
            policy: i32,
            suspend_count: i32,
        }
        const MACH_TASK_BASIC_INFO: i32 = 20;
        unsafe extern "C" {
            static mach_task_self_: u32;
            fn task_info(task: u32, flavor: i32, info: *mut MachTaskBasicInfo, count: *mut u32) -> i32;
        }
        let mut info = MachTaskBasicInfo::default();
        let mut count = (size_of::<MachTaskBasicInfo>() / size_of::<i32>()) as u32;
        // SAFETY: mach_task_self_ is the port of the current task; task_info writes at most `count` integers to the
        // valid out-pointer.
        let result = unsafe { task_info(mach_task_self_, MACH_TASK_BASIC_INFO, &mut info, &mut count) };
        (result == 0).then_some(info.resident_size)
    }
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
mod platform {
    use std::time::Duration;

    pub fn cpu_time() -> Option<Duration> {
        None
    }

    pub fn memory() -> Option<u64> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_process_has_cpu_time_and_memory() {
        let before = cpu_time().unwrap();
        let mut sum = 0u64;
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_millis(300) {
            sum = std::hint::black_box(sum.wrapping_add(1));
        }
        assert!(cpu_time().unwrap() > before);
        assert!(memory().unwrap() > 1024 * 1024);
    }

    #[test]
    fn the_platform_line() {
        let info = platform_info();
        assert!(info.ends_with(" cores)") || info.ends_with("(1 core)"), "{info}");
        #[cfg(all(windows, target_arch = "x86_64"))]
        assert!(info.starts_with("Windows x86_64("), "{info}");
    }

    #[test]
    fn dates_and_durations() {
        assert_eq!(civil_date(0), (1970, 1, 1));
        assert_eq!(civil_date(20_398), (2025, 11, 6));
        assert_eq!(civil_date(-1), (1969, 12, 31));
        assert_eq!(civil_date(11_016), (2000, 2, 29));
        assert_eq!(
            hms(Duration::from_millis((3 * 3600 + 25 * 60 + 7) * 1000 + 42)),
            "03:25:07.042"
        );
        assert_eq!(hms(Duration::from_secs(100 * 3600)), "100:00:00.000");
        assert_eq!(local_timestamp(0), "0");
        let stamp = local_timestamp(now_millis());
        assert_eq!(stamp.len(), "2026-11-06 18:06:50.123".len(), "{stamp}");
    }
}
