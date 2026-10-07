//! Minimal cdylib for the project-loader handshake tests.
use std::sync::atomic::{AtomicUsize, Ordering};

static INIT_CALLS: AtomicUsize = AtomicUsize::new(0);

#[unsafe(no_mangle)]
pub extern "C" fn cljrs_init_abi_fixture(_registry: *mut ()) {
    INIT_CALLS.fetch_add(1, Ordering::SeqCst);
}

#[unsafe(no_mangle)]
pub extern "C" fn test_init_calls() -> usize {
    INIT_CALLS.load(Ordering::SeqCst)
}

#[cfg(not(legacy))]
#[unsafe(no_mangle)]
pub extern "C" fn cljrs_dylib_abi() -> *const std::os::raw::c_char {
    #[cfg(matching)]
    const ABI: &str = concat!(env!("CLJRS_TEST_ABI"), "\0");
    #[cfg(not(matching))]
    const ABI: &str = "cljrs incompatible; rustc other; debug\0";
    ABI.as_ptr().cast()
}
