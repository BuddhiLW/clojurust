//! Regression test: `assoc-in` must preserve the existing keys (and metadata)
//! of its target map when that map carries metadata. Previously the match
//! was on the raw `WithMeta`-wrapped value instead of the unwrapped value,
//! so a metadata-carrying map fell through to the "not a map" branch and
//! `assoc-in` silently discarded every existing key, returning a fresh map
//! containing only the newly assoc'd path.

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
fn assoc_in_preserves_existing_keys_on_map_with_metadata() {
    let count = eval_fresh(
        "(count (assoc-in (with-meta {:tag-name \"h1\" :children []} {:parent :root}) [:on :click] :handler))",
    );
    assert_eq!(
        count,
        Value::Long(3),
        "assoc-in must keep the map's existing keys, not just the new path"
    );
}

#[test]
fn assoc_in_preserves_metadata_on_map() {
    let meta_val = eval_fresh(
        "(:parent (meta (assoc-in (with-meta {:tag-name \"h1\"} {:parent :root}) [:on :click] :handler)))",
    );
    assert_eq!(
        meta_val,
        Value::keyword(cljrs_value::Keyword::simple("root"))
    );
}

#[test]
fn assoc_in_without_metadata_still_works() {
    let count = eval_fresh("(count (assoc-in {:a {:b 1}} [:a :c] 2))");
    assert_eq!(count, Value::Long(1));
}
