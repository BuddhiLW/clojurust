//! Rust ↔ Clojure interop layer for clojurust.
//!
//! This crate provides the building blocks for Rust code to interact with
//! the clojurust runtime:
//!
//! - **`NativeObject`** re-export from `cljrs-value` — trait for opaque Rust
//!   structs wrapped as Clojure values
//! - **`FromValue` / `IntoValue`** — type-safe conversion between `Value` and
//!   Rust types
//! - **`wrap_result`** — convert `Result<T, E>` to `ValueResult<Value>`
//! - **`wrap_fn*`** — helpers to register Rust functions with automatic
//!   argument marshalling
//! - **`#[export]`** — proc-macro for automatic function registration
//! - **`register_exports`** — register all `#[export]`-annotated functions at once

pub mod error;
pub mod exports;
pub mod marshal;
pub mod register;
pub mod registry;

// Re-export the core interop traits from cljrs-value so downstream crates
// only need to depend on cljrs-interop.
pub use cljrs_gc::{GcPtr, MarkVisitor, Trace};
pub use cljrs_value::native_object::{NativeObject, NativeObjectBox, gc_native_object};
pub use cljrs_value::{Arity, NativeFn, Value, ValueError, ValueResult};

pub use error::wrap_result;
pub use exports::{ExportEntry, ProvenanceEntry, register_exports};
pub use marshal::{FromValue, IntoValue};
pub use register::{wrap_fn_variadic, wrap_fn0, wrap_fn1, wrap_fn2, wrap_fn3};
pub use registry::{InitFn, Registry};

// Re-export the proc-macro so users write `#[cljrs_interop::export(...)]`.
pub use cljrs_export_macro::export;

// Re-export inventory so the generated `::cljrs_interop::inventory::submit!`
// path resolves correctly inside user crates.
#[doc(hidden)]
pub use inventory;

/// Fingerprint shared with the pinned-package ABI handshake. The interop crate
/// is built in the extension's graph, so these values describe that graph's
/// cljrs version, compiler, and profile rather than the host process's.
pub fn abi_fingerprint() -> &'static str {
    if cfg!(debug_assertions) {
        concat!("cljrs ", env!("CARGO_PKG_VERSION"), "; ", env!("CLJRS_DYLIB_RUSTC"), "; debug")
    } else {
        concat!("cljrs ", env!("CARGO_PKG_VERSION"), "; ", env!("CLJRS_DYLIB_RUSTC"), "; release")
    }
}

/// Export a project init function together with the pinned-loader ABI symbol.
///
/// `export_init!(cljrs_init_my_crate, |registry: &mut cljrs_interop::Registry| {
///     // Register native functions here.
/// });`
/// The body runs only after the host checks the fingerprint. Use one invocation
/// per cdylib; the ABI symbol has a fixed name.
#[macro_export]
macro_rules! export_init {
    ($name:ident, $init:expr) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn cljrs_dylib_abi() -> *const ::std::os::raw::c_char {
            static ABI: ::std::sync::OnceLock<::std::ffi::CString> = ::std::sync::OnceLock::new();
            ABI.get_or_init(|| ::std::ffi::CString::new($crate::abi_fingerprint()).expect("ABI fingerprint contains NUL"))
                .as_ptr()
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn $name(registry: *mut $crate::Registry) {
            // SAFETY: the host verifies cljrs_dylib_abi before invoking this symbol.
            let registry = unsafe { &mut *registry };
            ($init)(registry);
        }
    };
}
