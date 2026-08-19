//! `(:refer-clojure ...)` in `ns`: `:exclude`, `:only` and `:rename` narrow the
//! automatic `clojure.core` refer.

use std::sync::Arc;

use cljrs_reader::Parser;
use cljrs_runtime::env::env::{Env, GlobalEnv};
use cljrs_runtime::env::error::{EvalError, EvalResult};
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

#[allow(clippy::result_large_err)]
fn try_eval_all(env: &mut Env, src: &str) -> EvalResult {
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_runtime::interp::eval::eval(&form, env)?;
    }
    Ok(result)
}

fn eval_all(env: &mut Env, src: &str) -> Value {
    try_eval_all(env, src).expect("eval error")
}

/// Assert that evaluating `src` fails because `sym` cannot be resolved.
fn assert_unbound(env: &mut Env, src: &str, sym: &str) {
    match try_eval_all(env, src) {
        Err(EvalError::UnboundSymbol(s)) => assert_eq!(s, sym),
        other => panic!("expected {sym} to be unresolved, got {other:?}"),
    }
}

#[test]
fn exclude_removes_the_core_refer() {
    let (_, mut env) = make_env();
    eval_all(&mut env, "(ns rc.exclude (:refer-clojure :exclude [inc]))");
    assert_unbound(&mut env, "(inc 1)", "inc");
    // Only the excluded name is affected; the rest of core is still referred.
    assert_eq!(eval_all(&mut env, "(dec 1)"), Value::Long(0));
    // The fully qualified var is still reachable.
    assert_eq!(eval_all(&mut env, "(clojure.core/inc 1)"), Value::Long(2));
}

#[test]
fn excluded_name_can_be_redefined_locally() {
    let (_, mut env) = make_env();
    eval_all(
        &mut env,
        "(ns rc.redef (:refer-clojure :exclude [inc]))
         (defn inc [x] (+ x 100))",
    );
    assert_eq!(eval_all(&mut env, "(inc 1)"), Value::Long(101));
    assert_eq!(eval_all(&mut env, "(clojure.core/inc 1)"), Value::Long(2));
}

#[test]
fn only_refers_just_the_listed_names() {
    let (_, mut env) = make_env();
    eval_all(&mut env, "(ns rc.only (:refer-clojure :only [+ str]))");
    assert_eq!(eval_all(&mut env, "(+ 1 2)"), Value::Long(3));
    assert_eq!(
        eval_all(&mut env, r#"(str "a" "b")"#),
        Value::string("ab".to_string())
    );
    assert_unbound(&mut env, "(inc 1)", "inc");
}

#[test]
fn rename_refers_under_the_new_name_only() {
    let (_, mut env) = make_env();
    eval_all(
        &mut env,
        "(ns rc.rename (:refer-clojure :rename {inc plus1}))",
    );
    assert_eq!(eval_all(&mut env, "(plus1 41)"), Value::Long(42));
    // As in `clojure.core/refer`, the original name is not also referred.
    assert_unbound(&mut env, "(inc 1)", "inc");
    assert_eq!(eval_all(&mut env, "(dec 1)"), Value::Long(0));
}

#[test]
fn exclude_and_only_combine() {
    let (_, mut env) = make_env();
    eval_all(
        &mut env,
        "(ns rc.both (:refer-clojure :only [+ inc] :exclude [inc]))",
    );
    assert_eq!(eval_all(&mut env, "(+ 1 2)"), Value::Long(3));
    assert_unbound(&mut env, "(inc 1)", "inc");
    assert_unbound(&mut env, "(dec 1)", "dec");
}

#[test]
fn require_refer_overrides_the_exclusion() {
    let (_, mut env) = make_env();
    // An explicit `:refer` names the var directly, so it wins over the
    // narrowed automatic core refer.
    eval_all(
        &mut env,
        "(ns rc.override
           (:refer-clojure :exclude [inc])
           (:require [clojure.core :refer [inc]]))",
    );
    assert_eq!(eval_all(&mut env, "(inc 1)"), Value::Long(2));
}

#[test]
fn reevaluating_ns_without_the_clause_restores_core() {
    let (_, mut env) = make_env();
    eval_all(&mut env, "(ns rc.reload (:refer-clojure :exclude [inc]))");
    assert_unbound(&mut env, "(inc 1)", "inc");
    eval_all(&mut env, "(ns rc.reload)");
    assert_eq!(eval_all(&mut env, "(inc 1)"), Value::Long(2));
}

#[test]
fn other_namespaces_are_unaffected() {
    let (_, mut env) = make_env();
    eval_all(&mut env, "(ns rc.private (:refer-clojure :exclude [inc]))");
    eval_all(&mut env, "(ns rc.other)");
    assert_eq!(eval_all(&mut env, "(inc 1)"), Value::Long(2));
}

#[test]
fn unknown_option_is_an_error() {
    let (_, mut env) = make_env();
    let err = try_eval_all(&mut env, "(ns rc.bad (:refer-clojure :bogus [inc]))")
        .expect_err("expected an error");
    assert!(
        format!("{err}").contains(":bogus"),
        "unexpected error: {err}"
    );
}

#[test]
fn exclude_expects_a_vector_of_symbols() {
    let (_, mut env) = make_env();
    let err = try_eval_all(&mut env, "(ns rc.bad2 (:refer-clojure :exclude inc))")
        .expect_err("expected an error");
    assert!(
        format!("{err}").contains(":exclude"),
        "unexpected error: {err}"
    );
}
