//! Deprecated compatibility shim.
//!
//! The tree-walking interpreter moved into
//! [`cljrs_runtime::interp`](../cljrs_runtime/interp/index.html) in Stage 2 of
//! the crate consolidation plan. This package only re-exports it so downstream
//! packages can migrate one at a time; it is removed in Stage 6.
//!
//! Replace `cljrs_interp::x` with `cljrs_runtime::interp::x`.

pub use cljrs_runtime::interp::*;
