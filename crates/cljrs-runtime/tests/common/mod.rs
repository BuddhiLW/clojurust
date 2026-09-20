//! One runtime per test thread, one namespace per case.
//!
//! A property suite that calls `Runtime::builder().build()` per case pays for
//! a whole bootstrap per case: the builder evaluates `bootstrap.cljrs`, and
//! with `eager_clojure_test` it evaluates `clojure.test` too, whose defmethods
//! cost milliseconds each. At a few hundred generated cases that is the entire
//! run time of the suite — `defonce_metadata_properties` spent 36s of a 193
//! second suite doing nothing but re-bootstrapping, and at the worst point in
//! its history (before the startup fixes) it was 97s.
//!
//! What a case actually needs isolating is its *vars*, not the whole runtime.
//! Vars live in a namespace, so a fresh namespace per case gives the same
//! isolation as a fresh runtime for anything that `def`s, `defonce`s or
//! `defn`s a name — at the cost of a hash-map insert instead of a bootstrap.
//!
//! The runtime is `thread_local` rather than a `OnceLock`: values live behind
//! `GcPtr`, which is `!Send`, so one shared runtime across the harness's test
//! threads is not merely unwise but does not compile. Each test thread builds
//! one and reuses it for every case it runs.
//!
//! **This is not for tests about startup.** A suite asserting what a *fresh*
//! runtime contains, or how long building one takes, must keep building one —
//! see `runtime_startup_canary.rs`, which is the suite deliberately left
//! paying that cost so a startup regression stays visible.

#![allow(dead_code)] // each test binary uses a different subset

use std::cell::Cell;
use std::sync::Arc;

use cljrs_reader::Parser;
use cljrs_runtime::env::env::{Env, GlobalEnv};
use cljrs_value::Value;

thread_local! {
    /// Built once per test thread, on first use.
    static GLOBALS: Arc<GlobalEnv> = build_globals_in(cljrs_runtime::ExecutionMode::TreeWalk);

    /// The tiered runtime is kept apart rather than rebuilt per caller: a
    /// suite that renders the same matrix through both tiers wants one of
    /// each, not one per row.
    static TIERED_GLOBALS: Arc<GlobalEnv> =
        build_globals_in(cljrs_runtime::ExecutionMode::Tiered);

    /// Distinguishes the namespaces handed out on this thread. Thread-local
    /// too, so two threads cannot collide: each has its own runtime, and a
    /// namespace name means nothing outside the runtime holding it.
    static NEXT_NS: Cell<u64> = const { Cell::new(0) };
}

fn build_globals_in(mode: cljrs_runtime::ExecutionMode) -> Arc<GlobalEnv> {
    cljrs_runtime::Runtime::builder()
        .execution_mode(mode)
        .eager_clojure_test(true)
        .build()
        .expect("runtime")
        .into_globals()
}

/// This thread's tree-walking runtime.
pub fn shared_globals() -> Arc<GlobalEnv> {
    GLOBALS.with(|g| g.clone())
}

/// This thread's runtime for `mode`.
///
/// A tiered runtime is built on first use, which matters for a caller that
/// flips a global lowering switch first: the switch is read when the runtime
/// starts compiling, not when this function is declared.
pub fn shared_globals_in(mode: cljrs_runtime::ExecutionMode) -> Arc<GlobalEnv> {
    match mode {
        cljrs_runtime::ExecutionMode::TreeWalk => GLOBALS.with(|g| g.clone()),
        _ => TIERED_GLOBALS.with(|g| g.clone()),
    }
}

/// An `Env` in a namespace no other case has used, with `clojure.core`
/// referred into it — so `inc`, `=` and the rest resolve exactly as they do in
/// `user`, and a `def` here is invisible to every other case.
pub fn fresh_env() -> (Arc<GlobalEnv>, Env) {
    fresh_env_in(cljrs_runtime::ExecutionMode::TreeWalk)
}

/// [`fresh_env`], over this thread's runtime for `mode`.
pub fn fresh_env_in(mode: cljrs_runtime::ExecutionMode) -> (Arc<GlobalEnv>, Env) {
    let globals = shared_globals_in(mode);
    let n = NEXT_NS.with(|c| {
        let n = c.get();
        c.set(n + 1);
        n
    });
    let ns = format!("prop-case-{n}");
    // Creates the namespace as well as referring into it.
    globals.refer_core(&ns);
    let env = Env::new(globals.clone(), &ns);
    (globals, env)
}

/// An `Env` in `ns`, emptied first: the namespace is dropped and recreated,
/// so nothing a previous caller defined in it survives.
///
/// This is for a caller that needs isolation AND a *stable* namespace name.
/// Auto-resolved keywords are the reason it exists: `::kw` reads as
/// `:<current-ns>/kw`, so a suite recording what `::kw` evaluates to would
/// otherwise record a different namespace name in every row and never match
/// its golden file twice.
pub fn reset_env_in(mode: cljrs_runtime::ExecutionMode, ns: &str) -> (Arc<GlobalEnv>, Env) {
    let globals = shared_globals_in(mode);
    globals.namespaces.write().unwrap().remove(ns);
    // Recreates the namespace as well as referring into it.
    globals.refer_core(ns);
    let env = Env::new(globals.clone(), ns);
    (globals, env)
}

/// Evaluate every form in `src`; yield the last value.
pub fn eval_in(env: &mut Env, src: &str) -> Result<Value, String> {
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().map_err(|e| format!("parse: {e:?}"))?;
    let mut result = Value::Nil;
    for form in forms {
        result =
            cljrs_runtime::interp::eval::eval(&form, env).map_err(|e| format!("eval: {e:?}"))?;
    }
    Ok(result)
}

/// Evaluate `src` in a namespace of its own.
pub fn eval_fresh(src: &str) -> Result<Value, String> {
    let (_g, mut env) = fresh_env();
    eval_in(&mut env, src)
}
