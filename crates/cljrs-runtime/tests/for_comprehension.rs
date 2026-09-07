//! `for` as a real list comprehension.
//!
//! It used to read only the first binding pair of its vector and drop
//! everything after it, so `(for [x (range 2) y (range 2)] [x y])` failed with
//! "unbound symbol: y" and no modifier was supported at all. `doseq` next to it
//! had handled multiple bindings and `:let`/`:when`/`:while` all along, which
//! is what made the gap easy to miss: the two macros document the same binding
//! grammar.
//!
//! These tests pin the grammar, the iteration order, the laziness, and the one
//! placement of `:while` the expansion cannot express.

use std::sync::Arc;

use cljrs_reader::Parser;
use cljrs_runtime::env::env::{Env, GlobalEnv};
use cljrs_value::Value;

fn make_env() -> (Arc<GlobalEnv>, Env) {
    let globals = cljrs_runtime::Runtime::builder()
        .execution_mode(cljrs_runtime::ExecutionMode::TreeWalk)
        .build()
        .expect("runtime")
        .into_globals();
    let env = Env::new(globals.clone(), "user");
    (globals, env)
}

fn eval_fresh(src: &str) -> Result<Value, String> {
    let (_, mut env) = make_env();
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().map_err(|e| format!("parse: {e:?}"))?;
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_runtime::interp::eval::eval(&form, &mut env)
            .map_err(|e| format!("eval: {e:?}"))?;
    }
    Ok(result)
}

/// Evaluate and render with `pr-str`, so a whole sequence is one comparison.
fn shows(src: &str) -> String {
    let wrapped = format!("(pr-str {src})");
    match eval_fresh(&wrapped).unwrap_or_else(|e| panic!("{src}\n{e}")) {
        Value::Str(s) => s.get().clone(),
        other => panic!("{src}: expected a string, got {other:?}"),
    }
}

// ── Bindings ─────────────────────────────────────────────────────────────────

#[test]
fn one_binding_still_works() {
    assert_eq!(shows("(vec (for [x (range 3)] x))"), "[0 1 2]");
    assert_eq!(shows("(vec (for [x [1 2 3]] (* x x)))"), "[1 4 9]");
}

#[test]
fn two_bindings_nest() {
    assert_eq!(
        shows("(vec (for [x (range 2) y (range 2)] [x y]))"),
        "[[0 0] [0 1] [1 0] [1 1]]"
    );
}

#[test]
fn the_rightmost_binding_varies_fastest() {
    assert_eq!(
        shows("(vec (for [c \"ab\" n (range 2)] (str c n)))"),
        "[\"a0\" \"a1\" \"b0\" \"b1\"]"
    );
}

#[test]
fn three_bindings_take_the_product() {
    assert_eq!(
        shows("(count (for [a (range 2) b (range 3) c (range 4)] [a b c]))"),
        "24"
    );
}

#[test]
fn an_empty_collection_yields_nothing() {
    assert_eq!(shows("(vec (for [x [] y (range 3)] x))"), "[]");
    assert_eq!(shows("(vec (for [x (range 3) y []] x))"), "[]");
}

#[test]
fn binding_forms_destructure() {
    assert_eq!(shows("(vec (for [[a b] [[1 2] [3 4]]] (+ a b)))"), "[3 7]");
    assert_eq!(
        shows("(vec (for [{:keys [n]} [{:n 1} {:n 2}]] n))"),
        "[1 2]"
    );
}

#[test]
fn an_inner_binding_can_use_an_outer_one() {
    assert_eq!(
        shows("(vec (for [x (range 4) y (range x)] [x y]))"),
        "[[1 0] [2 0] [2 1] [3 0] [3 1] [3 2]]"
    );
}

// ── Modifiers ────────────────────────────────────────────────────────────────

#[test]
fn when_skips_and_keeps_going() {
    assert_eq!(
        shows("(vec (for [x (range 10) :when (even? x)] x))"),
        "[0 2 4 6 8]"
    );
    assert_eq!(shows("(vec (for [x (range 5) :when false] x))"), "[]");
}

#[test]
fn let_binds_inside_the_loop() {
    assert_eq!(
        shows("(vec (for [x (range 3) :let [y (* 2 x)]] y))"),
        "[0 2 4]"
    );
}

#[test]
fn when_and_let_compose() {
    assert_eq!(
        shows("(vec (for [x (range 10) :when (even? x) :let [y (* x x)]] y))"),
        "[0 4 16 36 64]"
    );
}

#[test]
fn while_stops_rather_than_skipping() {
    // The distinction that matters: 8 is even, but :while has already ended
    // the loop at 5, so it never appears.
    assert_eq!(
        shows("(vec (for [x (range 10) :while (< x 5)] x))"),
        "[0 1 2 3 4]"
    );
    assert_eq!(
        shows("(vec (for [x [2 4 5 6 8] :while (even? x)] x))"),
        "[2 4]"
    );
}

#[test]
fn while_stops_only_its_own_loop() {
    assert_eq!(
        shows("(vec (for [x (range 2) y (range 5) :while (< y 2)] [x y]))"),
        "[[0 0] [0 1] [1 0] [1 1]]"
    );
}

#[test]
fn while_sees_the_lets_before_it() {
    assert_eq!(
        shows("(vec (for [x (range 10) :let [y (* x x)] :while (< y 9)] x))"),
        "[0 1 2]"
    );
}

#[test]
fn a_while_that_cannot_be_compiled_is_rejected_loudly() {
    // `:while` becomes a take-while over its own collection, which it can only
    // do while it still stands next to that collection. After a `:when` it
    // would have to stop a loop it can no longer see, so the macro refuses
    // instead of quietly behaving like `:when`.
    let err = eval_fresh("(for [x (range 3) :when (even? x) :while (< x 2)] x)")
        .expect_err("a :while after a :when should not expand");
    assert!(
        err.contains(":while must follow its binding"),
        "unhelpful error: {err}"
    );
}

// ── Laziness ─────────────────────────────────────────────────────────────────

#[test]
fn the_result_is_lazy() {
    // Realizing a million elements would not finish; taking three must.
    assert_eq!(
        shows("(vec (take 3 (for [x (range 1000000)] x)))"),
        "[0 1 2]"
    );
}

#[test]
fn laziness_survives_nesting() {
    assert_eq!(
        shows("(vec (take 3 (for [x (range 1000000) y (range 2)] [x y])))"),
        "[[0 0] [0 1] [1 0]]"
    );
}

// ── Body ─────────────────────────────────────────────────────────────────────

#[test]
fn a_nil_body_yields_nils_rather_than_vanishing() {
    // `for` returns one element per surviving combination; a nil element is
    // still an element. Dropping them would make `(for [...] (when p x))`
    // silently behave like a filter.
    assert_eq!(shows("(vec (for [x (range 2)] nil))"), "[nil nil]");
    assert_eq!(
        shows("(vec (for [x (range 3)] (when (even? x) x)))"),
        "[0 nil 2]"
    );
}

#[test]
fn a_multi_form_body_runs_in_order_and_yields_the_last() {
    assert_eq!(
        shows("(vec (for [x (range 2)] (inc x) (* 10 x)))"),
        "[0 10]"
    );
}
