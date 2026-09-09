//! `defmulti`'s optional docstring and attribute map.
//!
//! Clojure's signature is `(defmulti name docstring? attr-map? dispatch-fn &
//! options)`. Only the docstring was recognised, so an attribute map was taken
//! as the dispatch function — and since a map IS callable, nothing failed at
//! definition time. `(defmulti area "Doc." {:added "1.0"} :kind)` dispatched
//! every call through `{:added "1.0"}`, which returns nil for any shape, so the
//! failure surfaced later as "no method for dispatch value nil".

use std::sync::Arc;

use cljrs_reader::Parser;
use cljrs_runtime::env::env::{Env, GlobalEnv};
use cljrs_value::{Keyword, Value};

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
fn eval_fresh(src: &str) -> Result<Value, String> {
    let (_, mut env) = make_env();
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().map_err(|e| format!("parse: {e:?}"))?;
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_runtime::interp::eval::eval(&form, &mut env)
            .map_err(|e| format!("eval: {e:?}"))?;
    }
    Ok(result)
}

fn value_of(src: &str) -> Value {
    eval_fresh(src).unwrap_or_else(|e| panic!("{src}\n{e}"))
}

fn kw(name: &str) -> Value {
    Value::keyword(Keyword::simple(name))
}

fn text(s: &str) -> Value {
    Value::string(s.to_string())
}

/// The four head shapes, each dispatching on `:kind`.
const HEADS: [&str; 4] = [
    r#"(defmulti area :kind)"#,
    r#"(defmulti area "Area of a shape." :kind)"#,
    r#"(defmulti area {:added "1.0"} :kind)"#,
    r#"(defmulti area "Area of a shape." {:added "1.0"} :kind)"#,
];

#[test]
fn every_head_shape_dispatches_on_the_dispatch_fn() {
    for head in HEADS {
        let src = format!(
            r#"{head}
               (defmethod area :square [s] (* (:side s) (:side s)))
               (area {{:kind :square :side 4}})"#
        );
        assert_eq!(
            value_of(&src),
            Value::Long(16),
            "dispatch function not recognised in: {head}"
        );
    }
}

#[test]
fn an_attr_map_is_not_taken_as_the_dispatch_function() {
    // The regression, stated directly: a map in attr-map position dispatched
    // every call to nil, so every call fell through to :default.
    let src = r#"(defmulti area "Doc." {:added "1.0"} :kind)
                 (defmethod area :square [_] :square-method)
                 (defmethod area :default [_] :fell-through)
                 (area {:kind :square})"#;
    assert_eq!(value_of(src), kw("square-method"));
}

#[test]
fn the_docstring_lands_in_the_var_metadata() {
    for head in [HEADS[1], HEADS[3]] {
        let src = format!(r#"{head} (:doc (meta (var area)))"#);
        assert_eq!(value_of(&src), text("Area of a shape."), "in: {head}");
    }
}

#[test]
fn the_attr_map_lands_in_the_var_metadata() {
    for head in [HEADS[2], HEADS[3]] {
        let src = format!(r#"{head} (:added (meta (var area)))"#);
        assert_eq!(value_of(&src), text("1.0"), "in: {head}");
    }
}

#[test]
fn a_head_with_neither_carries_neither() {
    let src = format!(
        r#"{} [(:doc (meta (var area))) (:added (meta (var area)))]"#,
        HEADS[0]
    );
    assert_eq!(value_of(&src).to_string(), "[nil nil]");
}

#[test]
fn options_after_the_dispatch_fn_still_parse() {
    // `:default` is found relative to the dispatch fn, whose position moves
    // when a docstring or attr map precedes it.
    let src = r#"(defmulti area "Doc." {:added "1.0"} :kind :default :fallback)
                 (defmethod area :fallback [_] :used-the-fallback)
                 (area {:kind :hexagon})"#;
    assert_eq!(value_of(src), kw("used-the-fallback"));
}

#[test]
fn a_map_is_still_usable_as_the_dispatch_function() {
    // With no form following it, a map is the dispatch fn rather than an attr
    // map: `{:a :first}` looks its argument up and dispatches on the result.
    let src = r#"(defmulti lookup {:a :first})
                 (defmethod lookup :first [_] :found-first)
                 (lookup :a)"#;
    assert_eq!(value_of(src), kw("found-first"));
}

#[test]
fn a_defmulti_with_no_dispatch_function_is_rejected() {
    let err = eval_fresh("(defmulti area)").expect_err("should require a dispatch function");
    assert!(err.contains("dispatch function"), "unhelpful error: {err}");
}

#[test]
fn the_attr_map_beats_metadata_on_the_name() {
    // Precedence, not merely presence. Clojure's `(conj (meta mm-name) m)`
    // puts the attr map on top of the name's `^` marks, so a key present in
    // both takes the attr map's value.
    let src = r#"(defmulti ^{:added "name"} area {:added "attr"} :kind)
                 (:added (meta (var area)))"#;
    assert_eq!(value_of(src), text("attr"));
}

#[test]
fn the_docstring_beats_a_doc_key_in_the_attr_map() {
    let src = r#"(defmulti area "from the docstring" {:doc "from the attr map"} :kind)
                 (:doc (meta (var area)))"#;
    assert_eq!(value_of(src), text("from the docstring"));
}

#[test]
fn a_trailing_string_is_a_docstring_not_a_dispatch_function() {
    // A string is never callable, so taking it as the dispatch fn only defers
    // the error to the first call, pointing at the wrong thing. Clojure reads
    // a leading string as the docstring unconditionally.
    let err = eval_fresh(r#"(defmulti area "Area.")"#)
        .expect_err("a lone docstring leaves no dispatch function");
    assert!(err.contains("dispatch function"), "unhelpful error: {err}");
}
