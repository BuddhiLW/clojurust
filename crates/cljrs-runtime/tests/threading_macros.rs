//! Regression tests for the `some->`, `some->>`, and `run!` bootstrap
//! macros/fns (bootstrap.cljrs).

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

fn eval_src(src: &str) -> Value {
    let (_, mut env) = make_env();
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_runtime::interp::eval::eval(&form, &mut env).expect("eval error");
    }
    result
}

#[test]
fn some_threads_while_non_nil() {
    assert_eq!(eval_src("(some-> 1 inc (+ 2))"), Value::Long(4));
}

#[test]
fn some_short_circuits_on_nil() {
    assert_eq!(eval_src("(some-> nil inc (+ 2))"), Value::Nil);
    assert_eq!(eval_src("(some-> {:a 1} :b :c)"), Value::Nil);
}

#[test]
fn some_double_arrow_threads_while_non_nil() {
    assert_eq!(eval_src("(some->> 1 (+ 2) (* 3))"), Value::Long(9));
}

#[test]
fn some_double_arrow_short_circuits_on_nil() {
    assert_eq!(eval_src("(some->> nil (+ 2) (* 3))"), Value::Nil);
}

#[test]
fn run_bang_calls_proc_for_side_effects_and_returns_nil() {
    assert_eq!(
        eval_src("(let [a (atom 0)] (run! (fn [x] (swap! a + x)) [1 2 3]) @a)"),
        Value::Long(6)
    );
    assert_eq!(eval_src("(run! identity [1 2 3])"), Value::Nil);
}
