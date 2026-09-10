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

#[test]
fn a_multimethod_reached_through_refer_needs_no_qualification() {
    // The one path where the unqualified branch does real work rather than
    // just preserving same-ns behaviour: `:refer` puts the name in the
    // current namespace, so a bare `defmethod` has to find it there.
    let extender = r#"
(ns shapes.referred (:require [shapes.core :refer [area]]))
(defmethod area :triangle [t] (:base t))
"#;
    let (_dir, _g, mut env) = env_with_sources(&[
        ("shapes/core.cljrs", OWNER),
        ("shapes/referred.cljrs", extender),
    ]);
    eval_in(&mut env, "(require 'shapes.referred)").expect("require the extending ns");
    let v = eval_in(
        &mut env,
        "(do (require '[shapes.core :as c]) (c/area {:kind :triangle :base 9}))",
    )
    .expect("dispatch");
    assert_eq!(v, Value::Long(9));
}

#[test]
fn a_private_multimethod_cannot_be_extended_from_another_namespace() {
    // `^:private` is refused here for the same reason `eval_symbol` refuses
    // it: the owner did not publish this name. Before the cross-ns fix this
    // failed for the wrong reason ("is not a multimethod"), so widening the
    // lookup without the check would have quietly opened a hole.
    let owner = r#"
(ns shapes.hidden)
(defmulti ^:private secret :kind)
(defmethod secret :default [_] :owner-only)
"#;
    let extender = r#"
(ns shapes.intruder (:require [shapes.hidden :as h]))
(defmethod h/secret :x [_] :extended)
"#;
    let (_dir, _g, mut env) = env_with_sources(&[
        ("shapes/hidden.cljrs", owner),
        ("shapes/intruder.cljrs", extender),
    ]);
    let err = eval_in(&mut env, "(require 'shapes.intruder)")
        .expect_err("a private multimethod is not extensible from outside");
    assert!(err.contains("not public"), "unhelpful error: {err}");
}

#[test]
fn a_private_multimethod_is_still_extensible_by_its_owner() {
    // Privacy is about the boundary, not about the var: the owning namespace
    // extends its own multimethod normally.
    let owner = r#"
(ns shapes.own)
(defmulti ^:private secret :kind)
(defmethod secret :x [_] :extended)
(def result (secret {:kind :x}))
"#;
    let (_dir, _g, mut env) = env_with_sources(&[("shapes/own.cljrs", owner)]);
    eval_in(&mut env, "(require 'shapes.own)").expect("own-ns defmethod");
    assert_eq!(
        eval_in(&mut env, "shapes.own/result").expect("result"),
        Value::keyword(cljrs_value::Keyword::simple("extended"))
    );
}

#[test]
fn a_versioned_name_is_refused_rather_than_silently_unpinned() {
    // `Symbol::parse` splits `@hash` into `version`; reading only `name` would
    // extend HEAD while the caller believes they pinned a commit.
    let (_dir, _g, mut env) = env_with_sources(&[]);
    let err = eval_in(
        &mut env,
        "(do (defmulti f :k) (defmethod f@abc1234 :a [_] 1))",
    )
    .expect_err("a versioned defmethod target should be refused");
    assert!(err.contains("versioned"), "unhelpful error: {err}");
}

#[test]
fn a_pin_carried_in_the_namespace_half_is_refused_too() {
    // `(require '[mylib@abc1234 :as v1])` registers the namespace under its
    // literal versioned name and points the alias straight at it, so by the
    // time the owner ns is known the pin is inside it and `Symbol::parse` saw
    // no `@` at all. The state below is what that require leaves behind.
    let (_dir, globals, mut env) = env_with_sources(&[]);

    let mut pinned = Env::new(globals.clone(), "mylib@abc1234");
    // `require` registers the namespace and refers core into it; `Env::new`
    // only names one, and `refer_core` on a namespace that was never registered
    // returns silently. That gap was invisible while defmulti was a special
    // form, because a special form is ambient. It is a clojure.core macro now,
    // visible only where core was actually referred.
    globals.get_or_create_ns("mylib@abc1234");
    globals.refer_core("mylib@abc1234");
    eval_in(
        &mut pinned,
        "(defmulti render :kind) (defmethod render :default [_] :from-the-pin)",
    )
    .expect("defmulti in the versioned namespace");
    globals.add_alias("user", "v1", "mylib@abc1234");

    let err = eval_in(&mut env, "(defmethod v1/render :x [_] :extended)")
        .expect_err("extending a pinned multimethod should be refused");
    assert!(
        err.contains("versioned namespace"),
        "unhelpful error: {err}"
    );

    // And the pinned multimethod is untouched: the refusal happens before the
    // method table is reached.
    assert_eq!(
        eval_in(&mut env, "(v1/render {:kind :x})").expect("dispatch through the pin"),
        Value::keyword(cljrs_value::Keyword::simple("from-the-pin"))
    );
}

#[test]
fn a_versioned_namespace_can_still_extend_its_own_multimethods() {
    // The guard is about the boundary. While `mylib@abc1234` loads, its own
    // `defmethod`s must install normally -- `Env::new_versioned` sets
    // `current_ns` to the versioned name, so an unguarded check would break
    // exactly the loading path the pin exists to serve.
    let (_dir, globals, _env) = env_with_sources(&[]);
    let mut pinned = Env::new(globals.clone(), "mylib@abc1234");
    // `require` registers the namespace and refers core into it; `Env::new`
    // only names one, and `refer_core` on a namespace that was never registered
    // returns silently. That gap was invisible while defmulti was a special
    // form, because a special form is ambient. It is a clojure.core macro now,
    // visible only where core was actually referred.
    globals.get_or_create_ns("mylib@abc1234");
    globals.refer_core("mylib@abc1234");
    eval_in(
        &mut pinned,
        "(defmulti render :kind) (defmethod render :x [_] :own-method)",
    )
    .expect("a versioned namespace extending itself");
    assert_eq!(
        eval_in(&mut pinned, "(render {:kind :x})").expect("dispatch"),
        Value::keyword(cljrs_value::Keyword::simple("own-method"))
    );
}
