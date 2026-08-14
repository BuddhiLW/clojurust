//! Regression test: `vec` must accept a metadata-carrying collection argument
//! (previously it fell through to "wrong type: expected seq" because the
//! match was on the raw `WithMeta`-wrapped value instead of the unwrapped
//! value), and it must still strip any map-entry tagging from its result.

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
fn vec_accepts_metadata_carrying_vector() {
    let matches = eval_fresh("(= [1 2] (vec (with-meta [1 2] {:a 1})))");
    assert_eq!(matches, Value::Bool(true));
    let meta_val = eval_fresh("(:a (meta (vec (with-meta [1 2] {:a 1}))))");
    assert_eq!(meta_val, Value::Long(1));
}

#[test]
fn vec_accepts_metadata_carrying_list() {
    let matches = eval_fresh("(= [1 2] (vec (with-meta '(1 2) {:a 1})))");
    assert_eq!(matches, Value::Bool(true));
    let meta_val = eval_fresh("(:a (meta (vec (with-meta '(1 2) {:a 1}))))");
    assert_eq!(meta_val, Value::Long(1));
}

#[test]
fn vec_accepts_metadata_carrying_set() {
    let is_vec = eval_fresh("(vector? (vec (with-meta #{1 2} {:a 1})))");
    assert_eq!(is_vec, Value::Bool(true));
}

#[test]
fn vec_still_strips_map_entry_tag() {
    let is_map_entry = eval_fresh("(map-entry? (vec (map-entry :a 1)))");
    assert_eq!(is_map_entry, Value::Bool(false));
}

#[test]
fn vec_without_metadata_still_works() {
    let matches = eval_fresh("(= [1 2 3] (vec [1 2 3]))");
    assert_eq!(matches, Value::Bool(true));
}
