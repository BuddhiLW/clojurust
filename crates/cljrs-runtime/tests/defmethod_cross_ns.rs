//! `defmethod` on a multimethod that lives in another namespace.
//!
//! This is the normal shape of an open dispatch: one namespace owns the
//! `defmulti`, others extend it. `eval_defmethod` looked the name up with
//! `lookup_in_ns(current_ns, ..)` and nothing else, so a qualified name never
//! resolved and the error claimed a perfectly good multimethod "is not a
//! multimethod".
//!
//! The name now resolves the way `eval_symbol` and `binding` resolve one:
//! through the current namespace's `:require :as` aliases, then as a
//! fully-qualified namespace.

use std::path::PathBuf;
use std::sync::Arc;

use cljrs_reader::Parser;
use cljrs_runtime::env::env::{Env, GlobalEnv};
use cljrs_value::Value;

/// A runtime whose source path is a temp dir holding the given files.
fn env_with_sources(files: &[(&str, &str)]) -> (tempfile::TempDir, Arc<GlobalEnv>, Env) {
    let dir = tempfile::tempdir().expect("tempdir");
    for (rel, src) in files {
        let path = dir.path().join(rel);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, src).expect("write");
    }
    let globals = cljrs_runtime::Runtime::builder()
        .execution_mode(cljrs_runtime::ExecutionMode::TreeWalk)
        .source_paths(vec![PathBuf::from(dir.path())])
        .build()
        .expect("runtime")
        .into_globals();
    let env = Env::new(globals.clone(), "user");
    (dir, globals, env)
}

fn eval_in(env: &mut Env, src: &str) -> Result<Value, String> {
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().map_err(|e| format!("parse: {e:?}"))?;
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_runtime::interp::eval::eval(&form, env).map_err(|e| format!("{e:?}"))?;
    }
    Ok(result)
}

const OWNER: &str = r#"
(ns shapes.core)
(defmulti area "Area of a shape." :kind)
(defmethod area :default [_] :unknown)
"#;

#[test]
fn defmethod_reaches_a_multimethod_through_a_require_alias() {
    let extender = r#"
(ns shapes.square (:require [shapes.core :as core]))
(defmethod core/area :square [s] (* (:side s) (:side s)))
"#;
    let (_dir, _g, mut env) = env_with_sources(&[
        ("shapes/core.cljrs", OWNER),
        ("shapes/square.cljrs", extender),
    ]);
    eval_in(&mut env, "(require 'shapes.square)").expect("require the extending ns");
    let v = eval_in(
        &mut env,
        "(do (require '[shapes.core :as c]) (c/area {:kind :square :side 4}))",
    )
    .expect("dispatch");
    assert_eq!(v, Value::Long(16));
}

#[test]
fn defmethod_reaches_a_multimethod_by_its_full_namespace() {
    let extender = r#"
(ns shapes.circle (:require [shapes.core]))
(defmethod shapes.core/area :circle [c] (:r c))
"#;
    let (_dir, _g, mut env) = env_with_sources(&[
        ("shapes/core.cljrs", OWNER),
        ("shapes/circle.cljrs", extender),
    ]);
    eval_in(&mut env, "(require 'shapes.circle)").expect("require the extending ns");
    let v = eval_in(
        &mut env,
        "(do (require '[shapes.core :as c]) (c/area {:kind :circle :r 7}))",
    )
    .expect("dispatch");
    assert_eq!(v, Value::Long(7));
}

#[test]
fn several_namespaces_can_extend_the_same_multimethod() {
    // The point of an open dispatch: the owner does not know its extenders.
    let square = r#"
(ns shapes.square (:require [shapes.core :as core]))
(defmethod core/area :square [s] (* (:side s) (:side s)))
"#;
    let circle = r#"
(ns shapes.circle (:require [shapes.core :as core]))
(defmethod core/area :circle [c] (:r c))
"#;
    let (_dir, _g, mut env) = env_with_sources(&[
        ("shapes/core.cljrs", OWNER),
        ("shapes/square.cljrs", square),
        ("shapes/circle.cljrs", circle),
    ]);
    eval_in(&mut env, "(require 'shapes.square)").expect("square");
    eval_in(&mut env, "(require 'shapes.circle)").expect("circle");
    eval_in(&mut env, "(require '[shapes.core :as c])").expect("alias");

    assert_eq!(
        eval_in(&mut env, "(c/area {:kind :square :side 3})").expect("square dispatch"),
        Value::Long(9)
    );
    assert_eq!(
        eval_in(&mut env, "(c/area {:kind :circle :r 5})").expect("circle dispatch"),
        Value::Long(5)
    );
    assert_eq!(
        eval_in(&mut env, "(c/area {:kind :hexagon})").expect("default"),
        Value::keyword(cljrs_value::Keyword::simple("unknown"))
    );
}

#[test]
fn an_unqualified_name_still_resolves_in_the_current_namespace() {
    let (_dir, _g, mut env) = env_with_sources(&[]);
    let v = eval_in(
        &mut env,
        "(do (defmulti f :k) (defmethod f :a [_] 1) (f {:k :a}))",
    )
    .expect("same-ns defmethod");
    assert_eq!(v, Value::Long(1));
}

#[test]
fn extending_something_that_is_not_a_multimethod_says_so() {
    let (_dir, _g, mut env) = env_with_sources(&[]);
    let err = eval_in(&mut env, "(do (def g 1) (defmethod g :a [_] 1))")
        .expect_err("a def is not a multimethod");
    assert!(err.contains("not a multimethod"), "unhelpful error: {err}");
}

#[test]
fn extending_something_undefined_says_that_instead() {
    // Distinguishable from the above: "not defined" points at a missing
    // require, "not a multimethod" points at the wrong kind of var.
    let (_dir, _g, mut env) = env_with_sources(&[]);
    let err = eval_in(&mut env, "(defmethod nope/area :a [_] 1)").expect_err("no such multimethod");
    assert!(err.contains("not defined"), "unhelpful error: {err}");
}
