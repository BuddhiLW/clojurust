//! The clojurust runtime: environment, builtins, tree walker, and tiered evaluation.
//!
//! This package is the merge of four formerly separate packages. Each is now a
//! module:
//!
//! | Module | Former package | Responsibility |
//! |---|---|---|
//! | [`env`] | `cljrs-env` | Namespaces, vars, dynamic bindings, GC roots, loader |
//! | [`builtins`] | `cljrs-builtins` | Native `clojure.core` functions and bootstrap source |
//! | [`interp`] | `cljrs-interp` | Tree-walking interpreter, special forms, macros |
//! | [`tiered`] | `cljrs-eval` | IR lowering, tier-1 IR interpreter, JIT dispatch state |
//!
//! The four former packages remain as thin re-export shims for one migration
//! stage; new code should depend on `cljrs-runtime` and use these module paths.
//!
//! Construction goes through one path — [`Runtime::builder`] — and the
//! execution mode it selects ([`ExecutionMode`]) is what decides whether a
//! call tree-walks, runs lowered IR, or jumps to JIT-compiled native code.

// EvalError::Thrown wraps a full Value; boxing would require pervasive changes.
#![allow(clippy::result_large_err)]
// Namespace/GlobalEnv use Mutex<HashMap<Arc<str>, GcPtr<Var>>> — intentionally verbose for clarity.
#![allow(clippy::type_complexity)]
#![allow(clippy::arc_with_non_send_sync)]

pub mod builtins;
pub mod env;
pub mod interp;
pub mod mode;
pub mod runtime;
pub mod tiered;

pub use mode::{ExecutionMode, TierState};
pub use runtime::{BuildError, Runtime, RuntimeBuilder};
