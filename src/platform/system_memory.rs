//! Small, read-only OS memory probes. All quantities are bytes.
//!
//! These are advisory snapshots, not allocation guarantees or GPU-memory
//! measurements. Keep OS access off the rendering thread.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub total: u64,
    pub available: u64,
}

impl Snapshot {
    fn checked(total: u64, available: u64) -> Option<Self> {
        (total > 0).then_some(Self {
            total,
            available: available.min(total),
        })
    }
}

/// Returns None when the OS cannot provide a usable reading. Zero available
/// bytes is a valid, critical reading and must not be treated as probe failure.
pub fn snapshot() -> Option<Snapshot> {
    platform_snapshot()
}

#[cfg(windows)]
fn platform_snapshot() -> Option<Snapshot> {
    // MEMORYSTATUSEX from the public Win32 sysinfoapi.h ABI.
    #[repr(C)]
    #[derive(Default)]
    struct MemoryStatus {
        length: u32,
        load: u32,
        total_physical: u64,
        available_physical: u64,
        total_commit: u64,
        available_commit: u64,
        total_virtual: u64,
        available_virtual: u64,
        available_extended_virtual: u64,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GlobalMemoryStatusEx(status: *mut MemoryStatus) -> i32;
    }

    let mut status = MemoryStatus {
        length: std::mem::size_of::<MemoryStatus>() as u32,
        ..Default::default()
    };
    // SAFETY: status has the documented C layout, initialized size and a
    // writable buffer that remains alive for the entire synchronous call.
    if unsafe { GlobalMemoryStatusEx(&mut status) } == 0 {
        return None;
    }
    Snapshot::checked(
        status.total_physical,
        status
            .available_physical
            .min(status.available_commit)
            .min(status.available_virtual),
    )
}

#[cfg(target_os = "linux")]
fn platform_snapshot() -> Option<Snapshot> {
    // MemAvailable includes reclaimable memory and kernel reserves. MemFree
    // alone would incorrectly classify a healthy file cache as pressure.
    // This desktop probe reports host RAM, not per-container cgroup limits.
    parse_meminfo(&std::fs::read_to_string("/proc/meminfo").ok()?)
}

#[cfg(any(target_os = "linux", test))]
fn parse_meminfo(text: &str) -> Option<Snapshot> {
    fn bytes(text: &str, key: &str) -> Option<u64> {
        let value = text.lines().find_map(|line| line.strip_prefix(key))?;
        let mut fields = value.split_whitespace();
        let kb = fields.next()?.parse::<u64>().ok()?;
        if fields.next()? != "kB" || fields.next().is_some() {
            return None;
        }
        kb.checked_mul(1024)
    }
    Snapshot::checked(bytes(text, "MemTotal:")?, bytes(text, "MemAvailable:")?)
}

#[cfg(target_os = "macos")]
fn platform_snapshot() -> Option<Snapshot> {
    use std::{
        ffi::{c_char, c_void},
        ptr,
    };

    #[link(name = "System")]
    unsafe extern "C" {
        fn sysctlbyname(
            name: *const c_char,
            old: *mut c_void,
            old_len: *mut usize,
            new: *mut c_void,
            new_len: usize,
        ) -> i32;
        fn mach_host_self() -> u32;
        static mach_task_self_: u32;
        fn mach_port_deallocate(task: u32, name: u32) -> i32;
        fn host_page_size(host: u32, size: *mut usize) -> i32;
        fn host_statistics(host: u32, flavor: i32, data: *mut i32, count: *mut u32) -> i32;
    }

    let mut total = 0u64;
    let mut total_len = std::mem::size_of::<u64>();
    // SAFETY: the NUL-terminated key is immutable; the output buffer and its
    // length are valid. A null new pointer makes this a read-only sysctl.
    if unsafe {
        sysctlbyname(
            c"hw.memsize".as_ptr(),
            (&mut total as *mut u64).cast(),
            &mut total_len,
            ptr::null_mut(),
            0,
        )
    } != 0
        || total_len != std::mem::size_of::<u64>()
    {
        return None;
    }

    // HOST_VM_INFO has a stable 15-natural_t layout (vm_statistics_data_t).
    // Request that public legacy flavor, avoiding platform-dependent padding
    // and newer vm_statistics64 revisions. Its page gauges are still u32.
    const HOST_VM_INFO: i32 = 2;
    let mut pages = [0u32; 15];
    let mut count = pages.len() as u32;
    let mut page_size = 0usize;
    // SAFETY: mach_host_self returns a send right owned by this caller. Both
    // queries receive valid buffers; the right is released on every path.
    let (page_result, stats_result) = unsafe {
        let host = mach_host_self();
        if host == 0 {
            return None;
        }
        let page_result = host_page_size(host, &mut page_size);
        let stats_result =
            host_statistics(host, HOST_VM_INFO, pages.as_mut_ptr().cast(), &mut count);
        mach_port_deallocate(mach_task_self_, host);
        (page_result, stats_result)
    };
    if page_result != 0 || stats_result != 0 || count < 3 || page_size == 0 {
        return None;
    }
    // Approximate available RAM as free + inactive, including potentially
    // reclaimable cached pages. Speculative pages already belong to free;
    // purgeable pages overlap the queues and must not be added again.
    let available_pages = u64::from(pages[0]) + u64::from(pages[2]);
    Snapshot::checked(total, available_pages.saturating_mul(page_size as u64))
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn platform_snapshot() -> Option<Snapshot> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meminfo_uses_available_not_free() {
        let value = parse_meminfo(
            "MemTotal: 8388608 kB\nMemFree: 12 kB\nCached: 4096 kB\nMemAvailable: 3145728 kB\n",
        )
        .unwrap();
        assert_eq!(value.total, 8 * 1024 * 1024 * 1024);
        assert_eq!(value.available, 3 * 1024 * 1024 * 1024);
    }

    #[test]
    fn meminfo_zero_available_is_pressure_and_large_value_is_clamped() {
        assert_eq!(
            parse_meminfo("MemTotal: 8 kB\nMemAvailable: 0 kB\n"),
            Some(Snapshot {
                total: 8192,
                available: 0
            })
        );
        assert_eq!(
            parse_meminfo("MemTotal: 8 kB\nMemAvailable: 9 kB\n"),
            Some(Snapshot {
                total: 8192,
                available: 8192
            })
        );
    }

    #[test]
    fn meminfo_rejects_missing_malformed_and_overflowing_readings() {
        for input in [
            "MemTotal: 8 kB\nMemFree: 2 kB\n",
            "MemTotal: 0 kB\nMemAvailable: 0 kB\n",
            "MemTotal: 8 B\nMemAvailable: 1 kB\n",
            "MemTotal: 8 kB\nMemAvailable: invalid kB\n",
            "MemTotal: 18446744073709551615 kB\nMemAvailable: 1 kB\n",
        ] {
            assert_eq!(parse_meminfo(input), None, "{input}");
        }
    }

    #[test]
    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    fn native_snapshot_has_consistent_byte_units() {
        // A restricted sandbox may deny this probe. If available, verify the
        // ABI/unit contract without depending on the host's current RAM use.
        if let Some(value) = snapshot() {
            assert!(value.total >= 1024 * 1024);
            assert!(value.available <= value.total);
        }
    }
}
