//! Every lazy producer answers `realized?`.
//!
//! `realized?` throws on anything that is not pending, so a producer whose head
//! is a plain cons rather than a lazy seq is not merely "unrealized" — it is
//! un-probeable, and the compliance suite's `lazy-seq?` check (a `realized?`
//! probe in a try/catch) then reports it as not lazy at all.

use std::sync::Arc;

use cljrs_reader::Parser;
use cljrs_runtime::env::env::{Env, GlobalEnv};
use cljrs_value::Value;

fn make_env() -> (Arc<GlobalEnv>, Env) {
    let globals = cljrs_runtime::Runtime::builder()
        .execution_mode(cljrs_runtime::ExecutionMode::TreeWalk)
        .eager_clojure_test(true)
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

fn eval_ok(src: &str) -> Value {
    eval_fresh(src).unwrap_or_else(|e| panic!("{src}\n{e}"))
}

/// Producers whose result must be a pending lazy seq, not an already-built cons.
const LAZY_PRODUCERS: &[&str] = &[
    "(iterate inc 0)",
    "(map inc [1 2])",
    "(range)",
    "(range 3)",
    "(repeat :x)",
    "(repeatedly (fn [] 1))",
    "(cycle [1 2])",
    "(filter even? (range))",
    "(lazy-seq [1])",
];

#[test]
fn a_lazy_producer_starts_unrealized_and_ends_realized() {
    for producer in LAZY_PRODUCERS {
        assert_eq!(
            eval_ok(&format!("(realized? {producer})")),
            Value::Bool(false),
            "{producer} should be pending before anything is demanded"
        );
        assert_eq!(
            eval_ok(&format!("(let [s {producer}] (first s) (realized? s))")),
            Value::Bool(true),
            "{producer} should be realized once its head is demanded"
        );
    }
}

#[test]
fn iterate_does_not_call_f_before_the_second_element() {
    let v = eval_ok("(first (iterate (fn [_] (throw (ex-info \"eager!\" {}))) 0))");
    assert_eq!(v, Value::Long(0));
}

#[test]
fn iterate_still_produces_the_right_sequence() {
    assert_eq!(eval_ok("(nth (iterate inc 0) 4)"), Value::Long(4));
    assert_eq!(
        format!("{}", eval_ok("(take 5 (iterate inc 0))")),
        "(0 1 2 3 4)"
    );
    assert_eq!(
        format!("{}", eval_ok("(take 3 (iterate (fn [x] (* 2 x)) 1))")),
        "(1 2 4)"
    );
}

#[test]
fn realized_still_rejects_things_that_are_not_pending() {
    for eager in ["'()", "[]", "{}", "#{}", "1", ":foo"] {
        assert!(
            eval_fresh(&format!("(realized? {eager})")).is_err(),
            "realized? must throw on {eager}"
        );
    }
}
