//! `mapv` over more than one collection.
//!
//! Its docstring described the multi-collection behaviour ("the set of first
//! items of each coll") from the start, but only the two-argument arity was
//! defined, so `(mapv f xs ys)` was an arity error. `map` next to it already
//! carried every arity, which is what made the gap survive: the obvious
//! reading of the source says it works.

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

fn shows(src: &str) -> String {
    let wrapped = format!("(pr-str {src})");
    match eval_fresh(&wrapped).unwrap_or_else(|e| panic!("{src}\n{e}")) {
        Value::Str(s) => s.get().clone(),
        other => panic!("{src}: expected a string, got {other:?}"),
    }
}

#[test]
fn one_collection_still_works() {
    assert_eq!(shows("(mapv inc [1 2 3])"), "[2 3 4]");
}

#[test]
fn two_collections() {
    assert_eq!(shows("(mapv + [1 2] [10 20])"), "[11 22]");
}

#[test]
fn three_collections() {
    assert_eq!(
        shows("(mapv vector [1 2] [3 4] [5 6])"),
        "[[1 3 5] [2 4 6]]"
    );
}

#[test]
fn four_or_more_collections() {
    assert_eq!(shows("(mapv + [1] [2] [3] [4])"), "[10]");
    assert_eq!(shows("(mapv + [1] [2] [3] [4] [5])"), "[15]");
}

#[test]
fn it_stops_at_the_shortest_collection() {
    assert_eq!(shows("(mapv + [1 2 3] [1 2])"), "[2 4]");
    assert_eq!(shows("(mapv vector [1 2 3] [1] [1 2])"), "[[1 1 1]]");
}

#[test]
fn an_empty_collection_yields_an_empty_vector() {
    assert_eq!(shows("(mapv + [] [1 2])"), "[]");
}

#[test]
fn the_result_is_a_vector_not_a_seq() {
    // The whole point of `mapv` over `map`: it is realized and indexable.
    assert_eq!(shows("(vector? (mapv + [1 2] [3 4]))"), "true");
    assert_eq!(shows("(nth (mapv + [1 2] [3 4]) 1)"), "6");
}

#[test]
fn it_agrees_with_map_over_the_same_arguments() {
    for args in [
        "inc [1 2 3]",
        "+ [1 2] [10 20]",
        "vector [1 2] [3 4] [5 6]",
        "+ [1 2 3] [1 2]",
    ] {
        assert_eq!(
            shows(&format!("(mapv {args})")),
            shows(&format!("(vec (map {args}))")),
            "mapv and map disagree for ({args})"
        );
    }
}
