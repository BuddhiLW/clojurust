//! `clojure.rust.ffi`: open a C ABI shared library and call its symbols.
//!
//! ```clojure
//! (require '[clojure.rust.ffi :as ffi])
//! (def lib (ffi/open "libm.so.6"))
//! (def cos (ffi/function lib "cos" [:double] :double))
//! (cos 0.0) ;=> 1.0
//! (ffi/close lib)
//! ```
//!
//! No libffi: every signature is called through one of two function-pointer
//! shapes (see `docs/book/src/rust-interop/c-ffi.md`), which limits a
//! signature to 6 integer-class and 8 `:double` arguments, no varargs, no
//! structs by value. The namespace exists only on x86_64 and aarch64
//! non-Windows native targets; elsewhere [`init`] registers nothing.

use std::sync::Arc;

use cljrs_runtime::env::env::GlobalEnv;

/// The namespace this crate registers.
pub const NS: &str = "clojure.rust.ffi";

/// Whether this target has the namespace at all.
pub const SUPPORTED: bool = cfg!(all(
    any(target_arch = "x86_64", target_arch = "aarch64"),
    not(target_os = "windows"),
    not(target_family = "wasm")
));

#[cfg(all(
    any(target_arch = "x86_64", target_arch = "aarch64"),
    not(target_os = "windows"),
    not(target_family = "wasm")
))]
mod imp;

/// Register `clojure.rust.ffi` into `globals`.
///
/// Idempotent: the namespace is built only on the first call. A no-op on
/// targets where [`SUPPORTED`] is false.
pub fn init(globals: &Arc<GlobalEnv>) {
    #[cfg(all(
        any(target_arch = "x86_64", target_arch = "aarch64"),
        not(target_os = "windows"),
        not(target_family = "wasm")
    ))]
    {
        if globals.is_loaded(NS) {
            return;
        }
        globals.get_or_create_ns(NS);
        imp::register(globals);
        globals.mark_loaded(NS);
    }
    #[cfg(not(all(
        any(target_arch = "x86_64", target_arch = "aarch64"),
        not(target_os = "windows"),
        not(target_family = "wasm")
    )))]
    let _ = globals;
}
