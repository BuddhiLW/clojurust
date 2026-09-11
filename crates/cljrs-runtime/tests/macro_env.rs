//! `&env` reaches a macro that reads it, and is not built for one that cannot.
//!
//! `defmacro` records whether the body mentions `&env`; expansion builds the
//! map of every local in scope only then. The map cost about 5us per local on
//! every expansion, and the tree-walker expands a macro on every use, so a
//! `when` in a loop inside a wide `let` paid for a value nothing read.

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

fn eval_value(env: &mut Env, src: &str) -> Value {
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_runtime::interp::eval::eval(&form, env)
            .unwrap_or_else(|e| panic!("{src}\neval: {e:?}"));
    }
    result
}

fn pr(env: &mut Env, src: &str) -> String {
    match eval_value(env, &format!("(pr-str {src})")) {
        Value::Str(s) => s.get().as_str().to_string(),
        other => panic!("expected a string from pr-str, got a {}", other.type_name()),
    }
}

fn uses_env(env: &mut Env, macro_name: &str) -> bool {
    match eval_value(env, &format!("(deref (var {macro_name}))")) {
        Value::Macro(m) => m.get().macro_uses_env,
        other => panic!("{macro_name} is not a macro but a {}", other.type_name()),
    }
}

#[test]
fn a_macro_that_reads_env_sees_every_local_in_scope() {
    let (_g, mut env) = make_env();
    eval_value(
        &mut env,
        "(defmacro locals [] (vec (sort (map name (keys &env)))))",
    );
    assert!(uses_env(&mut env, "locals"));
    assert_eq!(
        pr(&mut env, "(let [b 2] (let [a 1] (locals)))"),
        "[\"a\" \"b\"]"
    );
    assert_eq!(pr(&mut env, "(locals)"), "[]");
}

#[test]
fn a_mention_anywhere_in_the_body_counts() {
    let (_g, mut env) = make_env();
    // Inside a nested `let`, inside a syntax-quote unquote, inside metadata:
    // every one is a mention, because the walk is total over the form.
    eval_value(&mut env, "(defmacro nested [] (let [e &env] (count e)))");
    eval_value(&mut env, "(defmacro spliced [] `(count ~(count &env)))");
    eval_value(&mut env, "(defmacro annotated [] (count ^{:x &env} []))");
    for m in ["nested", "spliced", "annotated"] {
        assert!(
            uses_env(&mut env, m),
            "{m} mentions &env and must be flagged"
        );
    }
    assert_eq!(pr(&mut env, "(let [a 1 b 2 c 3] (nested))"), "3");
}

#[test]
fn a_macro_that_never_mentions_env_is_not_flagged_and_still_expands() {
    let (_g, mut env) = make_env();
    eval_value(&mut env, "(defmacro twice [x] (list '* 2 x))");
    assert!(!uses_env(&mut env, "twice"));
    assert_eq!(pr(&mut env, "(let [a 21] (twice a))"), "42");
    // `&form` alone is not `&env`.
    eval_value(&mut env, "(defmacro shape [& _] (count &form))");
    assert!(!uses_env(&mut env, "shape"));
    assert_eq!(pr(&mut env, "(let [a 1] (shape a a a))"), "4");
}
