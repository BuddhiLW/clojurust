//! Deprecated compatibility shim.
//!
//! The namespace and environment layer moved into
//! [`cljrs_runtime::env`](../cljrs_runtime/env/index.html) in Stage 2 of the
//! crate consolidation plan. This package only re-exports it so downstream
//! packages can migrate one at a time; it is removed in Stage 6.
//!
//! Replace `cljrs_env::x` with `cljrs_runtime::env::x`.

pub use cljrs_runtime::env::*;
