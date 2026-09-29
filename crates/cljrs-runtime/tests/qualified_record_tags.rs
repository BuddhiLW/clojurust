//! Regression tests: a record or deftype's dispatch tag is qualified by the
//! namespace that defined it.
//!
//! The tag used to be the bare type name, so two namespaces that each defined
//! a `Point` shared one tag: the second namespace's protocol impls replaced the
//! first one's, and `instance?` could not tell the two types apart. This broke
//! hosting several addons in one runtime, where unrelated libraries choose
//! record names independently.

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

/// Evaluate `src` and return the last form's value as `pr-str` renders it.
fn eval_printed(src: &str) -> String {
    let (_, mut env) = make_env();
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_runtime::interp::eval::eval(&form, &mut env).expect("eval error");
    }
    match result {
        Value::Str(s) => s.get().clone(),
        other => panic!("expected a string from pr-str, got {other}"),
    }
}

const TWO_POINTS: &str = r#"
(ns proto.p)
(defprotocol Named (nm [x]))

(ns a.one)
(defrecord Point [x] proto.p/Named (nm [_] :one))

(ns b.two)
(defrecord Point [x])
(extend-type Point proto.p/Named (nm [_] :two))

(ns user)
"#;

#[test]
fn same_named_records_in_two_namespaces_keep_their_own_impls() {
    let src = format!(
        "{TWO_POINTS}(pr-str [(proto.p/nm (a.one/->Point 1)) (proto.p/nm (b.two/->Point 1))])"
    );
    assert_eq!(eval_printed(&src), "[:one :two]");
}

#[test]
fn instance_q_tells_same_named_records_apart() {
    let src = format!(
        "{TWO_POINTS}(pr-str [(instance? a.one/Point (a.one/->Point 1)) \
                              (instance? b.two/Point (a.one/->Point 1))])"
    );
    assert_eq!(eval_printed(&src), "[true false]");
}

#[test]
fn extend_protocol_resolves_a_record_through_its_namespace() {
    let src = r#"
(ns proto.q)
(defprotocol Sized (size [x]))
(ns c.three)
(defrecord Box [w])
(extend-protocol proto.q/Sized
  Box (size [b] (:w b))
  String (size [s] (count s)))
(ns user)
(pr-str [(proto.q/size (c.three/->Box 3)) (proto.q/size "abcd")])
"#;
    assert_eq!(eval_printed(src), "[3 4]");
}

#[test]
fn a_record_prints_with_its_qualified_type_name() {
    let src = format!("{TWO_POINTS}(pr-str (a.one/->Point 1))");
    assert_eq!(eval_printed(&src), "#a.one.Point{:x 1}");
}
