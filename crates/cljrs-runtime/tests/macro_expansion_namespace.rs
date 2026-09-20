//! What `*ns*` is while a macro expands.
//!
//! On the JVM a macro expands at COMPILE time, in the namespace being
//! compiled, so `*ns*` during expansion is the namespace the call was written
//! in — the same answer whether the call sits at the top level of a file or
//! inside a `deftest`, a `fn`, or any other body.
//!
//! Here it was not. `*ns*` is a dynamic var that `ns`/`in-ns` set, and nothing
//! rebinds it per call, so a macro expanded while a function RAN read whatever
//! the caller happened to leave there. Measured: at the top level of `probe.b`
//! it was `probe.b`, and inside a `deftest` body in the same file it was
//! `user`.
//!
//! The consequence is not that `*ns*` prints oddly. Every macro that resolves
//! a symbol during expansion resolves it in the wrong namespace, and gets
//! `nil` rather than an error. The live instance in this repo is
//! `defmethod-pin` in `bootstrap.cljrs`, which asks `(ns-aliases *ns*)`
//! whether a defmethod target is version-pinned: written inside a `deftest`,
//! it was judged against `user`'s aliases and silently reported no pin.
//!
//! Measuring it turned up something wider than the report. `*ns*` does not
//! track the namespace at all: it is synced only by `ns`, `in-ns` and the file
//! loader, so an `Env` created directly in a namespace reads `*ns*` as `user`
//! at its own TOP level. `*ns*` and `env.current_ns` are two sources of truth
//! for one fact, and they drift wherever nothing calls `in-ns`.
//!
//! These tests are `#[ignore]`d because they FAIL: they are the repro for
//! CLJRS-DEFTEST-NS, committed so the next person starts from a measurement
//! rather than from a description. Remove the `ignore` with the fix. The fix
//! direction that follows from the measurement: bind `*ns*` to the call site's
//! `env.current_ns` for the duration of a macro expansion in
//! `interp::macros::macroexpand_1`, which is the point where Clojure's answer
//! ("`*ns*` is the namespace being compiled") and this runtime's answer
//! diverge.

mod common;

use cljrs_value::Value;

/// `(str *ns*)` as a macro sees it, captured at three sites in one namespace.
fn ns_seen_at_each_site() -> (String, String, String, String) {
    let (_g, mut env) = common::fresh_env();
    let here = env.current_ns.to_string();

    common::eval_in(
        &mut env,
        r#"
        (require '[clojure.test :refer [deftest]])

        (defmacro ns-at-expansion [] (str *ns*))

        ;; 1. top level of this namespace
        (def at-top (ns-at-expansion))

        ;; 2. inside a plain fn body, which is the general case: the body is
        ;;    expanded when the fn runs, long after the file was read.
        (defn in-a-fn [] (ns-at-expansion))

        ;; 3. inside a deftest body, which is the case that was reported.
        (deftest in-a-deftest (def from-deftest (ns-at-expansion)))
        "#,
    )
    .expect("definitions evaluate");

    let top = match common::eval_in(&mut env, "at-top").expect("at-top") {
        Value::Str(s) => s.get().to_string(),
        other => panic!("expected a string, got {other:?}"),
    };

    let in_fn = match common::eval_in(&mut env, "(in-a-fn)").expect("in-a-fn") {
        Value::Str(s) => s.get().to_string(),
        other => panic!("expected a string, got {other:?}"),
    };

    // Run the test var the way the runner does, then read what it captured.
    common::eval_in(&mut env, "(in-a-deftest)").expect("the deftest body runs");
    let in_deftest = match common::eval_in(&mut env, "from-deftest").expect("from-deftest") {
        Value::Str(s) => s.get().to_string(),
        other => panic!("expected a string, got {other:?}"),
    };

    (here, top, in_fn, in_deftest)
}

/// The widest of the three: `*ns*` is wrong at the top level too, whenever the
/// namespace was entered without `ns`/`in-ns`.
#[test]
#[ignore = "repro for CLJRS-DEFTEST-NS: *ns* is synced by ns/in-ns only, not by env.current_ns"]
fn the_top_level_of_a_namespace_sees_that_namespace() {
    let (here, top, _in_fn, _in_deftest) = ns_seen_at_each_site();
    assert_eq!(
        top, here,
        "at the top level of {here}, a macro saw *ns* = {top}"
    );
}

#[test]
#[ignore = "repro for CLJRS-DEFTEST-NS: a deftest body expands at run time, in whatever *ns* the runner left"]
fn a_macro_inside_a_deftest_expands_in_the_defining_namespace() {
    let (_here, top, _in_fn, in_deftest) = ns_seen_at_each_site();
    assert_eq!(
        in_deftest, top,
        "a macro expanded inside a deftest body saw {in_deftest}, \
         but the same macro at the top level of the same namespace saw {top}"
    );
}

#[test]
#[ignore = "repro for CLJRS-DEFTEST-NS: a fn body expands when it runs, and *ns* is not rebound per call"]
fn a_macro_inside_any_fn_body_expands_in_the_defining_namespace() {
    let (_here, top, in_fn, _in_deftest) = ns_seen_at_each_site();
    assert_eq!(
        in_fn, top,
        "a macro expanded inside a fn body saw {in_fn}, \
         but the same macro at the top level of the same namespace saw {top}. \
         deftest is only the instance that was reported; the defect is general."
    );
}
