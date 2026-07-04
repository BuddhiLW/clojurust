//! Regression test: `into` must accept a metadata-carrying target and
//! preserve that metadata on the result (previously it fell through to
//! "wrong type: expected collection" because the match was on the raw
//! `WithMeta`-wrapped value instead of the unwrapped value).

use std::sync::Arc;

use cljrs_env::env::{Env, GlobalEnv};
use cljrs_reader::Parser;
use cljrs_value::Value;

fn make_env() -> (Arc<GlobalEnv>, Env) {
    let globals = cljrs_interp::standard_env(None, None, None);
    let env = Env::new(globals.clone(), "user");
    (globals, env)
}

fn eval_fresh(src: &str) -> Value {
    let (_, mut env) = make_env();
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_interp::eval::eval(&form, &mut env).expect("eval error");
    }
    result
}

#[test]
fn into_accepts_metadata_carrying_vector_target() {
    let elems_match = eval_fresh("(= [1 2] (into (with-meta [] {:p 1}) [1 2]))");
    assert_eq!(elems_match, Value::Bool(true));
    let is_vec = eval_fresh("(vector? (into (with-meta [] {:p 1}) [1 2]))");
    assert_eq!(is_vec, Value::Bool(true));
}

#[test]
fn into_preserves_metadata_on_vector_target() {
    let meta_val = eval_fresh("(:p (meta (into (with-meta [] {:p 1}) [1 2])))");
    assert_eq!(meta_val, Value::Long(1));
}

#[test]
fn into_preserves_metadata_on_map_target() {
    let meta_val = eval_fresh("(:p (meta (into (with-meta {} {:p 1}) [[:a 1]])))");
    assert_eq!(meta_val, Value::Long(1));
    let is_map = eval_fresh("(map? (into (with-meta {} {:p 1}) [[:a 1]]))");
    assert_eq!(is_map, Value::Bool(true));
}

#[test]
fn into_preserves_metadata_on_set_target() {
    let meta_val = eval_fresh("(:p (meta (into (with-meta #{} {:p 1}) [1 2])))");
    assert_eq!(meta_val, Value::Long(1));
    let is_set = eval_fresh("(set? (into (with-meta #{} {:p 1}) [1 2]))");
    assert_eq!(is_set, Value::Bool(true));
}

#[test]
fn into_preserves_metadata_on_list_target() {
    let meta_val = eval_fresh("(:p (meta (into (with-meta '() {:p 1}) [1 2])))");
    assert_eq!(meta_val, Value::Long(1));
    let is_list = eval_fresh("(list? (into (with-meta '() {:p 1}) [1 2]))");
    assert_eq!(is_list, Value::Bool(true));
}

#[test]
fn into_matches_empty_form_idiom_used_by_clojure_walk() {
    // clojure.walk relies on (into (empty form) ...) round-tripping metadata
    // and collection type through into.
    let is_vec =
        eval_fresh("(vector? (into (empty (with-meta [1 2 3] {:tag :x})) (map inc [1 2 3])))");
    assert_eq!(is_vec, Value::Bool(true));

    let has_tag = eval_fresh(
        "(= :x (:tag (meta (into (empty (with-meta [1 2 3] {:tag :x})) (map inc [1 2 3])))))",
    );
    assert_eq!(has_tag, Value::Bool(true));
}

#[test]
fn into_without_metadata_still_works() {
    let elems_match = eval_fresh("(= [1 2 3] (into [] [1 2 3]))");
    assert_eq!(elems_match, Value::Bool(true));
}
