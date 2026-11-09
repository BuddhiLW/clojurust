//! Physical memory installed on this machine, used to pick the default GC
//! soft limit.

/// Assumed when the platform cannot be asked: 4 GiB.
const FALLBACK_TOTAL: u64 = 4 * 1024 * 1024 * 1024;

/// Total physical memory in bytes, or [`FALLBACK_TOTAL`] when the platform
/// does not report it.
pub(crate) fn total() -> u64 {
    match physical() {
        Some(bytes) if bytes > 0 => bytes,
        _ => FALLBACK_TOTAL,
    }
}

#[cfg(target_vendor = "apple")]
fn physical() -> Option<u64> {
    let mut bytes: u64 = 0;
    let mut len = std::mem::size_of::<u64>();
    // SAFETY: the name is NUL-terminated, and `bytes`/`len` describe a
    // buffer of exactly the size `hw.memsize` writes.
    let rc = unsafe {
        libc::sysctlbyname(
            c"hw.memsize".as_ptr(),
            (&raw mut bytes).cast(),
            &raw mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    (rc == 0).then_some(bytes)
}

#[cfg(all(unix, not(target_vendor = "apple")))]
fn physical() -> Option<u64> {
    // SAFETY: sysconf takes no pointers and reports failure as -1.
    let pages = unsafe { libc::sysconf(libc::_SC_PHYS_PAGES) };
    // SAFETY: as above.
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if pages <= 0 || page_size <= 0 {
        return None;
    }
    (pages as u64).checked_mul(page_size as u64)
}

#[cfg(windows)]
fn physical() -> Option<u64> {
    #[repr(C)]
    struct MemoryStatusEx {
        length: u32,
        memory_load: u32,
        total_phys: u64,
        avail_phys: u64,
        total_page_file: u64,
        avail_page_file: u64,
        total_virtual: u64,
        avail_virtual: u64,
        avail_extended_virtual: u64,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GlobalMemoryStatusEx(buffer: *mut MemoryStatusEx) -> i32;
    }

    let mut status = MemoryStatusEx {
        length: std::mem::size_of::<MemoryStatusEx>() as u32,
        memory_load: 0,
        total_phys: 0,
        avail_phys: 0,
        total_page_file: 0,
        avail_page_file: 0,
        total_virtual: 0,
        avail_virtual: 0,
        avail_extended_virtual: 0,
    };
    // SAFETY: `status` is a MEMORYSTATUSEX with `length` set to its size, as
    // the call requires.
    let ok = unsafe { GlobalMemoryStatusEx(&raw mut status) };
    (ok != 0).then_some(status.total_phys)
}

#[cfg(not(any(unix, windows)))]
fn physical() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn total_is_a_plausible_amount_of_memory() {
        let bytes = super::total();
        assert!(bytes >= 64 * 1024 * 1024, "{bytes}");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn total_agrees_with_proc_meminfo() {
        let meminfo = std::fs::read_to_string("/proc/meminfo").unwrap();
        let kib: u64 = meminfo
            .lines()
            .find_map(|line| line.strip_prefix("MemTotal:"))
            .and_then(|rest| rest.trim().trim_end_matches("kB").trim().parse().ok())
            .unwrap();
        let reported = kib * 1024;
        let total = super::total();
        // The kernel reserves some pages MemTotal leaves out, so sysconf
        // reads slightly higher.
        assert!(total >= reported, "{total} < {reported}");
        assert!(total - reported < reported / 8, "{total} vs {reported}");
    }
}
