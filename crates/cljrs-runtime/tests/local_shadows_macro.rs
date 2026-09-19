//! Regression tests: a local binding shadows a core macro at call position.
//!
//! cljrs defines `doc` (and friends) as macros in bootstrap.cljrs, loaded into
//! clojure.core. Before this fix, `resolve_macro` looked the head symbol up in
//! the global namespace without consulting the local frames, so
//! (let [doc (fn [x] :local)] (doc 1)) expanded clojure.core/doc on the
//! literal argument and answered its return value (nil), never calling the
//! local. Clojure semantics: locals shadow macros — the compiler checks the
//! local scope before macroexpanding a call.

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
fn let_local_shadows_core_macro() {
    let result = eval_src("(let [doc (fn [x] :local)] (doc 1))");
    assert_eq!(result, Value::keyword(cljrs_value::Keyword::parse("local")));
}

#[test]
fn fn_param_shadows_core_macro() {
    let result = eval_src("((fn [doc] (doc 1)) (fn [x] :param))");
    assert_eq!(result, Value::keyword(cljrs_value::Keyword::parse("param")));
}

#[test]
fn local_named_like_macro_holding_a_var_calls_the_var() {
    // The exact shape that surfaced this: a let-bound var resolved by
    // ns-resolve must be CALLED, not macroexpanded away.
    let result = eval_src(
        "(defn document [& xs] (count xs)) \
         (let [doc (ns-resolve (find-ns 'user) 'document)] (doc :a :b :c))",
    );
    assert_eq!(result, Value::Long(3));
}

#[test]
fn core_macro_still_expands_without_a_local() {
    // doc expands and runs (its answer for these inputs is nil; the point is
    // expansion happened rather than an unbound-symbol error).
    let result = eval_src("(doc 'some-undefined-symbol)");
    assert_eq!(result, Value::Nil);
    // A macro whose expansion is observable in the value, not just in not
    // erroring: when expands to an if.
    let result = eval_src("(when true :expanded)");
    assert_eq!(result, Value::keyword(cljrs_value::Keyword::parse("expanded")));
}
