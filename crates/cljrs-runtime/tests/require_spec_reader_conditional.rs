//! Regression test: a `require` spec may carry a reader conditional in its
//! namespace slot.
//!
//! `[#?(:clj clojure.core :cljs cljs.core) :as core]` is the idiom every
//! clj+cljs `.cljc` library uses to alias the host core. `clojure.test.check`
//! opens with it. The option loop already resolved conditionals in the trailing
//! `:as` / `:refer` positions, but the first element did not get the same
//! treatment, so the spec read as `[:as core]` and was rejected with "require
//! spec: first element must be a symbol".
//!
//! Every case here goes through an `ns` form on purpose. That is the path that
//! parses raw `Form`s, so the conditional is still an unresolved `ReaderCond`
//! when the spec is read. A quoted `(require '[#?(…) :as core])` does NOT
//! exercise it: evaluating the quote resolves the conditional first, so the
//! spec parser only ever sees the selected branch.

use std::sync::Arc;

use cljrs_reader::Parser;
use cljrs_runtime::env::env::{Env, GlobalEnv};
use cljrs_value::Value;

fn make_env() -> (Arc<GlobalEnv>, Env) {
    let globals = cljrs_runtime::interp::standard_env(None, None, None);
    let env = Env::new(globals.clone(), "user");
    (globals, env)
}

/// Assert `result` is `42`, the marker every positive case evaluates to
/// through the alias, proving the aliased namespace actually resolved.
///
/// The aliased namespace is `clojure.core`, which is exactly the shape
/// test.check uses (`[#?(:clj clojure.core :cljs cljs.core) :as core]`) and
/// the only one guaranteed present in a bare interpreter env: this harness
/// has no source path, so nothing loadable from disk can be relied on.
fn assert_marker(result: Value) {
    match result {
        Value::Long(n) => assert_eq!(n, 42),
        other => panic!("expected 42, got {other:?}"),
    }
}

fn eval_fresh(src: &str) -> Result<Value, String> {
    let (_, mut env) = make_env();
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().map_err(|e| format!("{e:?}"))?;
    let mut result = Value::Nil;
    for form in forms {
        result =
            cljrs_runtime::interp::eval::eval(&form, &mut env).map_err(|e| format!("{e:?}"))?;
    }
    Ok(result)
}

#[test]
fn namespace_slot_resolves_a_reader_conditional() {
    let result = eval_fresh(
        "(ns t.a (:require [#?(:rust clojure.core :default clojure.core) :as core])) \
         (core/inc 41)",
    )
    .expect("spec with a conditional namespace should load");
    assert_marker(result);
}

#[test]
fn default_branch_is_taken_when_no_platform_key_matches() {
    let result = eval_fresh(
        "(ns t.b (:require [#?(:clj clojure.set :cljs cljs.core :default clojure.core) :as core])) \
         (core/inc 41)",
    )
    .expect("the :default branch should be selected on :rust");
    assert_marker(result);
}

/// The trailing options already resolved conditionals; that must keep working
/// now that the head is resolved too.
#[test]
fn option_slot_conditionals_still_resolve() {
    let result = eval_fresh(
        "(ns t.c (:require [clojure.core #?(:cljs :refer-macros :default :as) core])) \
         (core/inc 41)",
    )
    .expect("a conditional in the option position is the case that already worked");
    assert_marker(result);
}

/// A plain spec must be untouched by the new resolution step.
#[test]
fn plain_namespace_slot_is_unaffected() {
    let result = eval_fresh("(ns t.d (:require [clojure.core :as core])) (core/inc 41)")
        .expect("plain spec should be unaffected");
    assert_marker(result);
}

/// When no branch matches there is no namespace to require, so the spec is
/// refused by name rather than reported as a malformed first element. The
/// message must name the real cause: `#?(:clj … :cljs …)` with no `:default`
/// is unloadable on this runtime, and the caller needs to know it is the
/// missing branch and not their spelling. `clojure.test.check` is exactly
/// this shape.
#[test]
fn no_matching_branch_is_reported_as_such() {
    let err = eval_fresh("(ns t.e (:require [#?(:clj clojure.core :cljs cljs.core) :as core]))")
        .expect_err("no branch matches :rust");
    assert!(
        err.contains("no reader-conditional branch matched"),
        "message should name the real cause, got: {err}"
    );
}
