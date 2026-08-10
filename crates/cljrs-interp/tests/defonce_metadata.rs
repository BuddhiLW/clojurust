//! Regression test: `defonce` must accept a metadata-carrying symbol, the way
//! `def` already does. It used to extract the name with a plain symbol match,
//! so `(defonce ^:private registry (atom {}))` failed with "defonce requires a
//! symbol at position 0" while the identical `def` form worked.

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
fn defonce_accepts_a_metadata_carrying_symbol() {
    let result = eval_fresh("(defonce ^:private registry (atom {})) (map? @registry)");
    assert_eq!(result, Value::Bool(true));
}

#[test]
fn defonce_keeps_the_metadata_on_the_var() {
    let result = eval_fresh("(defonce ^:private registry 1) (:private (meta #'registry))");
    assert_eq!(result, Value::Bool(true));
}

#[test]
fn defonce_with_metadata_still_binds_only_once() {
    let result = eval_fresh("(defonce ^:private v 1) (defonce ^:private v 2) v");
    assert_eq!(result, Value::Long(1));
}

#[test]
fn defonce_without_metadata_is_unchanged() {
    let result = eval_fresh("(defonce v 1) (defonce v 2) v");
    assert_eq!(result, Value::Long(1));
}

#[test]
fn defonce_still_rejects_a_non_symbol_name() {
    let (_, mut env) = make_env();
    let mut parser = Parser::new("(defonce 42 1)".to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    assert!(cljrs_interp::eval::eval(&forms[0], &mut env).is_err());
}
