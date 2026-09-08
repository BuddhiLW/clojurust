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

// ── Review findings (#377) ───────────────────────────────────────────────────

#[test]
fn a_binding_may_be_named_after_a_core_function() {
    // The expansion emits `list`, `mapcat`, `take-while` and `map` into the
    // same scope as the user's bindings, so an unqualified emission is callable
    // only until someone binds that name.
    assert_eq!(shows("(for [list [1 2]] list)"), "(1 2)");
    assert_eq!(shows("(for [x [1 2] :let [list x]] list)"), "(1 2)");
    assert_eq!(shows("(for [mapcat [[1] [2]] x mapcat] x)"), "(1 2)");
    assert_eq!(shows("(for [map [[1] [2]] x map] x)"), "(1 2)");
    assert_eq!(
        shows("(for [take-while [[1] [2]] x take-while] x)"),
        "(1 2)"
    );
    assert_eq!(shows("(for [some? [1 2] :while (< some? 2)] some?)"), "(1)");
}

#[test]
fn a_let_init_runs_once_per_element_under_while() {
    // The :let inits used to be folded into the take-while predicate AND
    // re-emitted into the body, so anything side-effecting ran twice.
    let src = r#"(let [n (atom 0)]
                   [(vec (for [x (range 4)
                               :let [y (do (swap! n inc) x)]
                               :while (< y 3)]
                           y))
                    @n])"#;
    assert_eq!(shows(src), "[[0 1 2] 4]");
}

#[test]
fn a_while_still_stops_while_an_inner_when_only_skips() {
    // Both compile to "produce nothing for this element"; only :while may end
    // the loop. The frame representation has to keep them distinguishable.
    assert_eq!(
        shows("(for [x (range 6) :while (< x 4) y [x] :when (even? y)] y)"),
        "(0 2)"
    );
    assert_eq!(
        shows("(for [x (range 6) :while (< x 4) :let [y (* x 10)]] y)"),
        "(0 10 20 30)"
    );
}

#[test]
fn a_malformed_binding_vector_is_rejected() {
    // Both used to yield an empty seq: `(second binds)` was nil and mapcat over
    // nil is empty, so a dropped collection expression reported nothing at all.
    for src in [
        "(for [x [1 2] y] x)",
        "(for [x [1 2] :when] x)",
        "(for [x] x)",
    ] {
        let err = eval_fresh(src).expect_err("should reject a malformed binding vector");
        assert!(
            err.contains("even number of forms"),
            "unhelpful error for {src}: {err}"
        );
    }
}

#[test]
fn the_innermost_simple_binding_is_one_to_one_and_lazy() {
    // The innermost binding with no modifiers now expands through `map` rather
    // than `mapcat` plus a per-element `list` — a per-element allocation, and
    // the pinned arity-2 `map` fast path in the IR interpreter and in codegen.
    // There is no `macroexpand` in this runtime to assert the shape with, so
    // pin the two properties the substitution has to preserve.
    assert_eq!(shows("(count (for [x (range 5)] x))"), "5");
    assert_eq!(shows("(for [x (range 3)] (inc x))"), "(1 2 3)");

    let src = r#"(let [n (atom 0)]
                   [(vec (take 2 (for [x (range 100)] (do (swap! n inc) x))))
                    @n])"#;
    assert_eq!(shows(src), "[[0 1] 2]", "the body must stay lazy");
}
