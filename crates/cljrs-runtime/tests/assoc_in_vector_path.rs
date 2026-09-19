//! Regression test: `assoc-in` must associate INTO a vector on its path, by
//! index, the way `assoc` does. It used to look a key up only in maps and
//! records, so a vector anywhere on the path was treated as "not a map" and
//! replaced by a fresh map keyed by the index:
//!
//!     (assoc-in {:t [{:c []} {:c []}]} [:t 0 :c] [:x])  =>  {:t {0 {:c [:x]}}}
//!
//! which silently dropped every other element of the vector.

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
    let forms = parser.parse_all().expect("parse error");
    let mut result = Value::Nil;
    for form in forms {
        result =
            cljrs_runtime::interp::eval::eval(&form, &mut env).map_err(|e| format!("{e:?}"))?;
    }
    Ok(result)
}

fn assert_true(src: &str) {
    assert_eq!(eval_fresh(src), Ok(Value::Bool(true)), "{src}");
}

#[test]
fn assoc_in_through_a_vector_keeps_the_vector_and_its_other_elements() {
    assert_true("(= {:t [{:c [:x]} {:c []}]} (assoc-in {:t [{:c []} {:c []}]} [:t 0 :c] [:x]))");
    assert_true("(vector? (:t (assoc-in {:t [{:c []} {:c []}]} [:t 1 :c] [:x])))");
}

#[test]
fn assoc_in_on_a_top_level_vector() {
    assert_true("(= [{:x 9}] (assoc-in [{:x 1}] [0 :x] 9))");
    assert_true("(= [1 5] (assoc-in [1 2] [1] 5))");
    assert_true("(vector? (assoc-in [1 2] [1] 5))");
}

#[test]
fn assoc_in_at_the_vector_count_appends_like_assoc() {
    assert_true("(= [1 2 3] (assoc-in [1 2] [2] 3))");
}

#[test]
fn assoc_in_past_the_vector_count_is_an_error_like_assoc() {
    assert!(eval_fresh("(assoc-in [1] [5] 0)").is_err());
}

#[test]
fn assoc_in_keeps_a_vector_s_metadata() {
    assert_true(
        "(let [r (assoc-in (with-meta [[0]] {:m 1}) [0 0] 2)] (and (= [[2]] r) (vector? r) (= {:m 1} (meta r))))",
    );
}

#[test]
fn assoc_in_still_creates_maps_for_missing_keys() {
    assert_true("(= {:a {:b {:c 1}}} (assoc-in {} [:a :b :c] 1))");
    assert_true("(= {:a {:b 1}} (assoc-in nil [:a :b] 1))");
}
