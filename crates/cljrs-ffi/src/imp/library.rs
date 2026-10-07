//! The opaque library handle `open` returns.
//!
//! The `libloading::Library` lives in an `Arc` shared by the handle and by
//! every function `function` built from it, so a bound function keeps its
//! library mapped even after the handle itself is collected. `close` empties
//! the slot: the library is unmapped once no call is in flight, and every
//! later use throws `{:ffi/error :closed}` instead of jumping into freed text.

use std::any::Any;
use std::fmt;
use std::sync::{Arc, RwLock, RwLockReadGuard};

use cljrs_gc::{MarkVisitor, Trace};
use cljrs_value::{NativeObject, ValueResult};

use super::error::{closed_error, open_error, symbol_error};

pub(crate) struct LibInner {
    pub(crate) path: String,
    lib: RwLock<Option<libloading::Library>>,
}

/// An open library. A held read guard pins it open for the call's duration.
pub(crate) struct OpenGuard<'a>(RwLockReadGuard<'a, Option<libloading::Library>>);

impl OpenGuard<'_> {
    fn library(&self) -> &libloading::Library {
        self.0
            .as_ref()
            .expect("OpenGuard is only built over an open library")
    }
}

impl LibInner {
    pub(crate) fn open(path: &str) -> ValueResult<Arc<Self>> {
        // SAFETY: loading a library runs its initialisers; that is the
        // capability this namespace grants, and the policy denies it in
        // transactions.
        let lib = unsafe { libloading::Library::new(path) }
            .map_err(|e| open_error(path, e.to_string()))?;
        Ok(Arc::new(LibInner {
            path: path.to_string(),
            lib: RwLock::new(Some(lib)),
        }))
    }

    pub(crate) fn guard(&self) -> ValueResult<OpenGuard<'_>> {
        let g = self.lib.read().unwrap_or_else(|e| e.into_inner());
        if g.is_none() {
            return Err(closed_error());
        }
        Ok(OpenGuard(g))
    }

    pub(crate) fn close(&self) {
        let mut g = self.lib.write().unwrap_or_else(|e| e.into_inner());
        g.take();
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.lib.read().unwrap_or_else(|e| e.into_inner()).is_none()
    }

    /// Address of `name`, resolved while the library is open.
    pub(crate) fn symbol(&self, name: &str) -> ValueResult<usize> {
        let guard = self.guard()?;
        // SAFETY: the symbol is only read as an address here; it is called
        // later through a signature the caller declared.
        let sym: libloading::Symbol<'_, *const ()> = unsafe {
            guard
                .library()
                .get(name.as_bytes())
                .map_err(|e| symbol_error(name, e.to_string()))?
        };
        Ok(*sym as usize)
    }
}

/// The Clojure-visible handle value (`Value::NativeObject`).
pub(crate) struct LibHandle(pub(crate) Arc<LibInner>);

impl fmt::Debug for LibHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = if self.0.is_closed() { "closed" } else { "open" };
        write!(f, "#<FfiLibrary {} {state}>", self.0.path)
    }
}

impl Trace for LibHandle {
    fn trace(&self, _: &mut MarkVisitor) {}
}

impl NativeObject for LibHandle {
    fn type_tag(&self) -> &str {
        "FfiLibrary"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
