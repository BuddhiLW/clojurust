//! Runs the clojure.test namespaces under `test/` against the compiled C
//! fixture, whose path reaches them as `CLJRS_FFI_FIXTURE`.

mod common;

use std::path::PathBuf;

use cljrs_value::Value;
use common::{env_with, eval_str, fixture_lib};

const TEST_NSES: &[&str] = &["clojure.rust.ffi-test"];

fn counter(map: &Value, key: &str) -> i64 {
    let mut found = 0i64;
    if let Value::Map(m) = map {
        m.for_each(|k, v| {
            if let (Value::Keyword(kw), Value::Long(n)) = (k, v)
                && kw.get().name.as_ref() == key
            {
                found = *n;
            }
        });
    }
    found
}

#[test]
fn run_clojure_ffi_tests() {
    let test_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test");
    // SAFETY: this test binary holds exactly one test, so nothing reads the
    // environment concurrently.
    unsafe { std::env::set_var("CLJRS_FFI_FIXTURE", fixture_lib()) };

    let _mutator = cljrs_gc::register_mutator();
    let mut env = env_with(vec![test_dir]);
    cljrs_runtime::env::callback::push_eval_context(&env);
    eval_str(&mut env, "(require 'clojure.test)");

    let (mut tests, mut failures) = (0i64, Vec::new());
    for ns in TEST_NSES {
        eval_str(&mut env, &format!("(require '{ns})"));
        let r = eval_str(&mut env, &format!("(clojure.test/run-tests '{ns})"));
        tests += counter(&r, "test");
        let (fail, error) = (counter(&r, "fail"), counter(&r, "error"));
        eprintln!(
            "[ffi-tests] {ns}: {} tests, {} pass, {fail} fail, {error} error",
            counter(&r, "test"),
            counter(&r, "pass")
        );
        if fail > 0 || error > 0 {
            failures.push(format!("{ns}: {fail} fail, {error} error"));
        }
    }
    cljrs_runtime::env::callback::pop_eval_context();

    assert!(tests > 0, "no tests ran");
    assert!(
        failures.is_empty(),
        "failures:\n  {}",
        failures.join("\n  ")
    );
}
