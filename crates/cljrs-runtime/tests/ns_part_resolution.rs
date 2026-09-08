//! One namespace-resolution rule, shared by every construct that needs it.
//!
//! `alias/name` and `fully.qualified.ns/name` must mean the same thing to a
//! symbol, a `var` form, a macro call, a syntax-quote, a `binding` target, a
//! protocol name and a `defmethod` target. The rule had been open-coded at
//! each of those sites, so "the same thing" was a coincidence maintained by
//! hand rather than a property — and a site added later (or a rule extended
//! later, as privacy and version pins were) simply missed whichever copies
//! nobody remembered.
//!
//! These pin the agreement itself: every construct below goes through
//! `Env::resolve_ns_part` / `Env::resolve_ns_or_current`.

use std::path::PathBuf;
use std::sync::Arc;

use cljrs_reader::Parser;
use cljrs_runtime::env::env::{Env, GlobalEnv};
use cljrs_value::Value;

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

/// One namespace, exercising every construct whose ns part must resolve.
const LIB: &str = r#"
(ns some.lib)
(def value 41)
(def ^:dynamic *knob* :default-knob)
(defmacro twice [x] (list 'clojure.core/+ x x))
(defprotocol Shaped (area [s]))
(defmulti describe :kind)
(defmethod describe :default [_] :unknown)
"#;

/// Evaluate `expr` twice — through an alias and through the full name — and
/// require the two to agree. `{ns}` is replaced by each spelling in turn.
fn agrees(expr_template: &str) -> Value {
    let (_dir, _g, mut env) = env_with_sources(&[("some/lib.cljrs", LIB)]);
    eval_in(&mut env, "(require '[some.lib :as l])").expect("require with an alias");

    let via_alias = eval_in(&mut env, &expr_template.replace("{ns}", "l"))
        .unwrap_or_else(|e| panic!("through the alias: {expr_template}\n{e}"));
    let via_full = eval_in(&mut env, &expr_template.replace("{ns}", "some.lib"))
        .unwrap_or_else(|e| panic!("through the full name: {expr_template}\n{e}"));

    assert_eq!(
        via_alias, via_full,
        "an alias and the full namespace name disagreed for: {expr_template}"
    );
    via_alias
}

#[test]
fn a_qualified_symbol_resolves_the_same_either_way() {
    assert_eq!(agrees("{ns}/value"), Value::Long(41));
}

#[test]
fn a_var_form_resolves_the_same_either_way() {
    assert_eq!(agrees("@(var {ns}/value)"), Value::Long(41));
}

#[test]
fn a_macro_call_resolves_the_same_either_way() {
    assert_eq!(agrees("({ns}/twice 21)"), Value::Long(42));
}

#[test]
fn a_syntax_quoted_symbol_resolves_the_same_either_way() {
    assert_eq!(
        agrees("(str `{ns}/value)"),
        Value::string("some.lib/value".to_string())
    );
}

#[test]
fn a_binding_target_resolves_the_same_either_way() {
    assert_eq!(
        agrees("(binding [{ns}/*knob* :bound] {ns}/*knob*)"),
        Value::keyword(cljrs_value::Keyword::simple("bound"))
    );
}

#[test]
fn a_protocol_name_resolves_the_same_either_way() {
    assert_eq!(
        agrees("(do (extend-protocol {ns}/Shaped Long (area [s] (* s 2))) (some.lib/area 5))"),
        Value::Long(10)
    );
}

#[test]
fn an_unknown_namespace_part_is_taken_literally() {
    // The fallback the seam has to preserve: a ns part that is not a known
    // alias names a namespace directly, whether or not it is loaded yet.
    let (_dir, _g, mut env) = env_with_sources(&[("some/lib.cljrs", LIB)]);
    eval_in(&mut env, "(require 'some.lib)").expect("require without an alias");
    assert_eq!(
        eval_in(&mut env, "some.lib/value").expect("full name with no alias in scope"),
        Value::Long(41)
    );
}

#[test]
fn an_unqualified_name_means_the_current_namespace() {
    let (_dir, _g, mut env) = env_with_sources(&[]);
    assert_eq!(
        eval_in(&mut env, "(do (def here 7) here)").expect("current-ns lookup"),
        Value::Long(7)
    );
}
