//! Metadata surviving the value -> form -> value round trip a macro makes.
//!
//! A macro's return value is converted back into a `Form` by `value_to_form`
//! before the expansion is evaluated. `attach_meta` produces `Value::WithMeta`
//! in quoted position, so without a matching arm on the way back the annotation
//! is dropped between the macro returning and its expansion running: a
//! `deftype` field loses its `^:unsynchronized-mutable`, and `(meta 'x)` on a
//! macro-produced symbol answers nil.

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

/// Evaluate every form in `src` in one fresh environment; yield the last value.
fn value_of(src: &str) -> Value {
    let (_, mut env) = make_env();
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser
        .parse_all()
        .unwrap_or_else(|e| panic!("{src}\nparse: {e:?}"));
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_runtime::interp::eval::eval(&form, &mut env)
            .unwrap_or_else(|e| panic!("{src}\neval: {e:?}"));
    }
    result
}

#[test]
fn metadata_on_a_macro_produced_symbol_survives_expansion() {
    let src = r#"(defmacro tagged [] (list 'quote (with-meta 'x {:tag "Long"})))
                 (:tag (meta (tagged)))"#;
    assert_eq!(value_of(src), Value::string("Long".to_string()));
}

#[test]
fn a_reader_flag_survives_expansion() {
    // The shape that motivated this: `^:unsynchronized-mutable field` reaching
    // the datatype special form through a macro expansion.
    let src = r#"(defmacro field [] (list 'quote (with-meta 'n {:unsynchronized-mutable true})))
                 (:unsynchronized-mutable (meta (field)))"#;
    assert_eq!(value_of(src), Value::Bool(true));
}
