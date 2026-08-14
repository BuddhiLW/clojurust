//! Deprecated compatibility shim.
//!
//! IR-accelerated (tiered) evaluation moved into
//! [`cljrs_runtime::tiered`](../cljrs_runtime/tiered/index.html) in Stage 2 of
//! the crate consolidation plan. This package only re-exports it so downstream
//! packages can migrate one at a time; it is removed in Stage 6.
//!
//! Replace `cljrs_eval::x` with `cljrs_runtime::tiered::x`.

pub use cljrs_runtime::tiered::*;
