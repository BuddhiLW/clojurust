//! Regression test: `empty` must preserve the metadata of its argument, and
//! must still return the correct empty collection type when the argument
//! carries metadata (previously it fell through to `nil` because the match
//! was on the raw `WithMeta`-wrapped value instead of the unwrapped value).

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

fn eval_fresh(src: &str) -> Value {
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
fn empty_preserves_metadata_on_vector() {
    let result = eval_fresh("(meta (empty (with-meta [1 2 3] {:a 1})))");
    assert!(
        !matches!(result, Value::Nil),
        "expected metadata to survive (empty ...), got nil"
    );
}

#[test]
fn empty_preserves_type_and_metadata_on_list() {
    let is_list = eval_fresh("(list? (empty (with-meta '(1 2 3) {:a 1})))");
    assert_eq!(is_list, Value::Bool(true));

    let meta_val = eval_fresh("(:a (meta (empty (with-meta '(1 2 3) {:a 1}))))");
    assert_eq!(meta_val, Value::Long(1));
}

#[test]
fn empty_preserves_type_and_metadata_on_map() {
    let is_map = eval_fresh("(map? (empty (with-meta {:x 1} {:a 1})))");
    assert_eq!(is_map, Value::Bool(true));

    let meta_val = eval_fresh("(:a (meta (empty (with-meta {:x 1} {:a 1}))))");
    assert_eq!(meta_val, Value::Long(1));
}

#[test]
fn empty_preserves_type_and_metadata_on_set() {
    let is_set = eval_fresh("(set? (empty (with-meta #{1 2} {:a 1})))");
    assert_eq!(is_set, Value::Bool(true));

    let meta_val = eval_fresh("(:a (meta (empty (with-meta #{1 2} {:a 1}))))");
    assert_eq!(meta_val, Value::Long(1));
}

#[test]
fn empty_without_metadata_still_works() {
    let cnt = eval_fresh("(count (empty [1 2 3]))");
    assert_eq!(cnt, Value::Long(0));
    let is_vec = eval_fresh("(vector? (empty [1 2 3]))");
    assert_eq!(is_vec, Value::Bool(true));
}
