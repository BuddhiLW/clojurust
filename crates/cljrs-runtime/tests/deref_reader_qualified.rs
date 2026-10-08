//! The `@x` reader macro means `clojure.core/deref`, fully qualified, as on
//! the JVM. A namespace that excludes `deref` and defines its own must not
//! capture `@x`, not even after the form passes through a macro (`reify`)
//! and comes back as data.

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

fn eval_all(env: &mut Env, src: &str) -> Value {
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_runtime::interp::eval::eval(&form, env).expect("eval error");
    }
    result
}

#[test]
fn quoted_deref_sugar_is_qualified() {
    let (_g, mut env) = make_env();
    let v = eval_all(&mut env, "(= '@a '(clojure.core/deref a))");
    assert_eq!(v, Value::Bool(true));
}

#[test]
fn deref_sugar_inside_reify_ignores_ns_local_deref() {
    let (_g, mut env) = make_env();
    let v = eval_all(
        &mut env,
        "(ns dq.excl (:refer-clojure :exclude [deref]))
         (defn deref [_] :ns-local)
         (defprotocol P (get-it [this]))
         (def a (atom 42))
         (get-it (reify P (get-it [_] @a)))",
    );
    assert_eq!(v, Value::Long(42));
}

#[test]
fn deref_sugar_inside_macro_ignores_ns_local_deref() {
    let (_g, mut env) = make_env();
    let v = eval_all(
        &mut env,
        "(ns dq.excl2 (:refer-clojure :exclude [deref]))
         (defn deref [_] :ns-local)
         (def a (atom 7))
         (when true @a)",
    );
    assert_eq!(v, Value::Long(7));
}
