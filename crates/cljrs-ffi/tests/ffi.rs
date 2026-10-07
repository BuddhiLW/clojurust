//! Rust-side checks of what clojure.test cannot see: the transaction policy
//! and the evaluator's error payloads.

mod common;

use cljrs_runtime::env::policy::TransactionPolicyGuard;
use cljrs_value::Value;
use common::{env_with, eval_str, fixture_lib, try_eval};

#[test]
fn ffi_is_denied_inside_a_transaction() {
    let _mutator = cljrs_gc::register_mutator();
    let mut env = env_with(vec![]);
    eval_str(&mut env, "(require '[clojure.rust.ffi :as ffi])");
    let lib = fixture_lib().display().to_string();
    for src in [
        format!("(ffi/open {lib:?})"),
        "(ffi/string 0)".to_string(),
        "(ffi/bytes 0 0)".to_string(),
    ] {
        let _policy = TransactionPolicyGuard::install();
        let err = try_eval(&mut env, &src).expect_err("ffi must be denied in a transaction");
        assert!(err.contains("ForbiddenEffect"), "{src}: {err}");
    }
}

#[test]
fn a_bound_function_refuses_calls_after_close() {
    let _mutator = cljrs_gc::register_mutator();
    let mut env = env_with(vec![]);
    eval_str(&mut env, "(require '[clojure.rust.ffi :as ffi])");
    let lib = fixture_lib().display().to_string();
    eval_str(&mut env, &format!("(def lib (ffi/open {lib:?}))"));
    eval_str(
        &mut env,
        "(def add (ffi/function lib \"add_l\" [:long :long] :long))",
    );
    assert_eq!(eval_str(&mut env, "(add 40 2)"), Value::Long(42));
    eval_str(&mut env, "(ffi/close lib)");
    eval_str(&mut env, "(ffi/close lib)");
    let err = try_eval(&mut env, "(add 40 2)").expect_err("closed");
    assert!(err.contains("closed"), "{err}");
}

#[test]
fn every_public_var_is_registered() {
    let _mutator = cljrs_gc::register_mutator();
    let mut env = env_with(vec![]);
    eval_str(&mut env, "(require '[clojure.rust.ffi :as ffi])");
    for name in [
        "open", "close", "sym", "function", "call", "string", "bytes",
    ] {
        let v = eval_str(&mut env, &format!("ffi/{name}"));
        assert!(matches!(v, Value::NativeFunction(_)), "{name}: {v:?}");
    }
}
