//! `deftype` / `defrecord` name a non-vector field list at the macro boundary.
//!
//! malli 0.20.2's `malli.impl.regex` writes deftype field vectors as
//! `#?(:clj [..] :cljs [..])` with no `:cljrs` or `:default` branch. On
//! clojurust that reads as nothing, the protocol name slides into the field
//! position, and loading used to fail deep inside with
//! "wrong type: expected seqable, got symbol", naming neither the type nor the
//! cause. The macros now refuse a non-vector field list with a message that
//! names the form, the type, what was found and the missing `:cljrs` branch.

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

/// Evaluate `src` and return the last value, which must be a string.
fn eval_str(src: &str) -> String {
    let (_globals, mut env) = make_env();
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_runtime::interp::eval::eval(&form, &mut env).expect("eval error");
    }
    match result {
        Value::Str(s) => s.get().as_str().to_string(),
        other => panic!("expected a string, got a {}", other.type_name()),
    }
}

const PROTO: &str = "(defprotocol ICache (ensure-cached! [c]))";

fn refusal(def: &str) -> String {
    eval_str(&format!(
        "{PROTO} (try (eval '{def}) \"no error\" (catch :default e (ex-message e)))"
    ))
}

#[test]
fn deftype_with_a_vanished_field_conditional_names_the_cause() {
    // The malli shape: the field vector is a conditional with no :cljrs branch.
    let msg = refusal(
        "(deftype Cache #?(:clj [values size] :cljs [values size]) ICache (ensure-cached! [_] 1))",
    );
    assert!(msg.contains("deftype Cache"), "{msg}");
    assert!(msg.contains("must be a vector"), "{msg}");
    assert!(msg.contains("ICache"), "{msg}");
    assert!(msg.contains(":cljrs"), "{msg}");
}

#[test]
fn defrecord_with_a_symbol_field_list_names_the_cause() {
    let msg = refusal("(defrecord Rec ICache (ensure-cached! [_] 1))");
    assert!(msg.contains("defrecord Rec"), "{msg}");
    assert!(msg.contains("must be a vector"), "{msg}");
}

#[test]
fn deftype_with_a_cljrs_field_branch_still_works() {
    let out = eval_str(&format!(
        "{PROTO} (deftype Cache #?(:clj [a] :cljrs [values]) ICache (ensure-cached! [_] values)) \
         (pr-str (ensure-cached! (->Cache 7)))"
    ));
    assert_eq!(out, "7");
}
