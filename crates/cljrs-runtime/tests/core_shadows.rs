//! `GlobalEnv::core_shadowed_names` — what a namespace binds that the IR
//! lowerer must not treat as `clojure.core` (issue #337).

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

fn eval_all(env: &mut Env, src: &str) -> Value {
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_runtime::interp::eval::eval(&form, env).expect("eval error");
    }
    result
}

#[test]
fn a_plain_namespace_shadows_nothing() {
    let (globals, mut env) = make_env();
    eval_all(&mut env, "(ns shadow.plain)");
    let shadows = globals.core_shadowed_names("shadow.plain");
    assert!(!shadows.contains("inc"), "{shadows:?}");
    assert!(!shadows.contains("count"), "{shadows:?}");
}

#[test]
fn a_local_def_shadows_the_core_name() {
    let (globals, mut env) = make_env();
    eval_all(&mut env, "(ns shadow.def) (defn inc [n] (+ n 7))");
    let shadows = globals.core_shadowed_names("shadow.def");
    assert!(shadows.contains("inc"), "{shadows:?}");
    assert!(!shadows.contains("dec"), "{shadows:?}");
}

#[test]
fn clojure_core_does_not_shadow_itself() {
    let (globals, _env) = make_env();
    let shadows = globals.core_shadowed_names("clojure.core");
    assert!(!shadows.contains("inc"), "{shadows:?}");
    assert!(!shadows.contains("count"), "{shadows:?}");
}

#[test]
fn refer_clojure_exclude_shadows_the_excluded_name() {
    let (globals, mut env) = make_env();
    eval_all(
        &mut env,
        "(ns shadow.exclude (:refer-clojure :exclude [inc]))",
    );
    let shadows = globals.core_shadowed_names("shadow.exclude");
    // Nothing binds `inc` here at all — so it is certainly not core's.
    assert!(shadows.contains("inc"), "{shadows:?}");
    assert!(!shadows.contains("dec"), "{shadows:?}");
}

#[test]
fn refer_clojure_rename_shadows_both_names() {
    let (globals, mut env) = make_env();
    eval_all(
        &mut env,
        "(ns shadow.rename (:refer-clojure :rename {inc bump}))",
    );
    let shadows = globals.core_shadowed_names("shadow.rename");
    // `inc` is no longer referred, and `bump` names a var whose name is `inc`.
    assert!(shadows.contains("inc"), "{shadows:?}");
    assert!(shadows.contains("bump"), "{shadows:?}");
    assert!(!shadows.contains("count"), "{shadows:?}");
}

#[test]
fn a_refer_from_another_namespace_shadows_the_core_name() {
    let (globals, mut env) = make_env();
    eval_all(&mut env, "(ns shadow.src) (defn count [c] :mine)");
    eval_all(&mut env, "(ns shadow.dst)");
    // What `(:require [shadow.src :refer [count]])` installs, without needing
    // the namespace to live on a source path.
    globals.refer_named("shadow.dst", "shadow.src", &[Arc::from("count")]);
    let shadows = globals.core_shadowed_names("shadow.dst");
    assert!(shadows.contains("count"), "{shadows:?}");
    assert!(!shadows.contains("first"), "{shadows:?}");
}
