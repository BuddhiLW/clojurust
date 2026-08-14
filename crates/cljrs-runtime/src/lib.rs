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

// EvalError::Thrown wraps a full Value; boxing would require pervasive changes.
#![allow(clippy::result_large_err)]
// Namespace/GlobalEnv use Mutex<HashMap<Arc<str>, GcPtr<Var>>> — intentionally verbose for clarity.
#![allow(clippy::type_complexity)]
#![allow(clippy::arc_with_non_send_sync)]

pub mod builtins;
pub mod env;
pub mod interp;
pub mod tiered;
