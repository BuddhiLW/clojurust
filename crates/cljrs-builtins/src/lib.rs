//! Deprecated compatibility shim.
//!
//! The native `clojure.core` builtins moved into
//! [`cljrs_runtime::builtins`](../cljrs_runtime/builtins/index.html) in Stage 2
//! of the crate consolidation plan. This package only re-exports them so
//! downstream packages can migrate one at a time; it is removed in Stage 6.
//!
//! Replace `cljrs_builtins::x` with `cljrs_runtime::builtins::x`.

pub use cljrs_runtime::builtins::*;
