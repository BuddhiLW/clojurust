//! `set!` must resolve a qualified target's namespace part the way everything
//! else does: through the current namespace's `:require … :as` alias table.
//!
//! `eval_set_bang` took the namespace part of the symbol LITERALLY —
//! `parsed.namespace.as_deref().unwrap_or(&env.current_ns)` — so after
//! `(:require [other.store :as st])`, reading `st/*slot*` worked and
//! `(set! st/*slot* v)` failed with "unbound symbol: st/*slot*". The full
//! spelling `(set! other.store/*slot* v)` worked, which is what makes it look
//! like a var problem rather than a resolution one.
//!
//! Same defect class as the `defmethod` cross-namespace bug and the protocol
//! impl-position bug PR #354 fixed: a user-supplied name resolved by string
//! instead of through the namespace. `resolve_protocol_sym` already calls
//! `env.resolve_ns_part`; this call site did not.

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

fn eval_in(env: &mut Env, src: &str) -> Result<Value, String> {
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().map_err(|e| format!("parse: {e:?}"))?;
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_runtime::interp::eval::eval(&form, env).map_err(|e| format!("{e}"))?;
    }
    Ok(result)
}

/// One environment: define the target namespace, alias it, then drive both
/// spellings of the same `set!` through it.
fn env_with_alias() -> (Arc<GlobalEnv>, Env) {
    let (globals, mut env) = make_env();
    eval_in(
        &mut env,
        "(in-ns 'other.store) (def ^:dynamic *slot* :initial) \
         (in-ns 'user) (alias 'st 'other.store)",
    )
    .expect("setup");
    (globals, env)
}

#[test]
fn set_bang_resolves_the_alias_in_a_qualified_target() {
    let (_g, mut env) = env_with_alias();
    let v = eval_in(
        &mut env,
        "(do (set! st/*slot* :aliased) other.store/*slot*)",
    )
    .expect("set! through an alias");
    assert_eq!(
        v,
        Value::keyword(cljrs_value::Keyword::simple("aliased")),
        "set! through the alias must reach the same var the full name does"
    );
}

#[test]
fn set_bang_still_accepts_the_full_namespace() {
    let (_g, mut env) = env_with_alias();
    let v = eval_in(&mut env, "(do (set! other.store/*slot* :full) st/*slot*)")
        .expect("set! through the full name");
    assert_eq!(v, Value::keyword(cljrs_value::Keyword::simple("full")));
}

#[test]
fn an_unknown_namespace_part_is_still_taken_literally_and_fails() {
    // Alias resolution must not invent a namespace: an unaliased, unknown ns
    // part stays literal, so the error still names what the user wrote.
    let (_g, mut env) = env_with_alias();
    let err = eval_in(&mut env, "(set! nope/*slot* :x)").expect_err("no such namespace");
    assert!(
        err.contains("nope/*slot*"),
        "the error should name the symbol as written: {err}"
    );
}
