//! `refer_core` on a namespace that does not exist yet creates it.
//!
//! It used to return silently, so a namespace referred before it was
//! registered ended up with `clojure.core` unbound in it, and nothing said so
//! until the first `inc` failed to resolve.

use std::sync::Arc;

use cljrs_reader::Parser;
use cljrs_runtime::env::env::{Env, GlobalEnv};
use cljrs_value::Value;

fn make_globals() -> Arc<GlobalEnv> {
    cljrs_runtime::Runtime::builder()
        .execution_mode(cljrs_runtime::ExecutionMode::TreeWalk)
        .build()
        .expect("runtime")
        .into_globals()
}

fn ns_exists(globals: &Arc<GlobalEnv>, ns: &str) -> bool {
    // Asked in Clojure from `user`: the namespace table is not public API.
    eval_in(globals, "user", &format!("(some? (find-ns '{ns}))")) == Ok(Value::Bool(true))
}

fn eval_in(globals: &Arc<GlobalEnv>, ns: &str, src: &str) -> Result<Value, String> {
    let mut env = Env::new(globals.clone(), ns);
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    let mut result = Value::Nil;
    for form in forms {
        result =
            cljrs_runtime::interp::eval::eval(&form, &mut env).map_err(|e| format!("{e:?}"))?;
    }
    Ok(result)
}

#[test]
fn referring_core_into_an_unregistered_namespace_creates_it_with_core_in_scope() {
    let globals = make_globals();
    assert!(
        !ns_exists(&globals, "fresh.ns"),
        "precondition: not registered"
    );

    globals.refer_core("fresh.ns");

    assert!(
        ns_exists(&globals, "fresh.ns"),
        "refer_core must register the namespace"
    );
    assert_eq!(
        eval_in(&globals, "fresh.ns", "(inc 41)"),
        Ok(Value::Long(42)),
        "core must be referred into the namespace it just created"
    );
}

#[test]
fn referring_core_twice_is_idempotent() {
    let globals = make_globals();
    globals.refer_core("twice.ns");
    // A def made between the two calls survives the second: the namespace is
    // kept, not replaced.
    assert!(eval_in(&globals, "twice.ns", "(def keep 7)").is_ok());
    globals.refer_core("twice.ns");
    assert_eq!(eval_in(&globals, "twice.ns", "keep"), Ok(Value::Long(7)));
    assert_eq!(eval_in(&globals, "twice.ns", "(inc 1)"), Ok(Value::Long(2)));
}
