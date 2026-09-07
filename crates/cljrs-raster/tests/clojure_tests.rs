//! Runs the Clojure-side `clojure.test` suite for the `cljrs.raster-test`
//! namespace against the native `cljrs.raster` functions registered by this crate.
//!
//! The test:
//! 1. Calls `cljrs_raster::register` to register the `cljrs.raster` namespace.
//! 2. Loads the standard environment with the crate's `test/` directory on
//!    the source path so `(require 'cljrs.raster-test)` resolves.
//! 3. Drives `clojure.test/run-tests` and fails if any assertion fails.

use std::path::PathBuf;

use cljrs_interop::Registry;
use cljrs_runtime::tiered::{Env, eval};
use cljrs_value::Value;

const TEST_NSES: &[&str] = &["cljrs.raster-test"];

fn parse_one(src: &str) -> cljrs_reader::Form {
    let mut parser = cljrs_reader::Parser::new(src.to_string(), "<test-driver>".to_string());
    let forms = parser.parse_all().expect("test driver: parse failed");
    forms
        .into_iter()
        .next()
        .expect("test driver: expected at least one form")
}

fn eval_str(env: &mut Env, src: &str) -> Value {
    let form = parse_one(src);
    eval(&form, env).unwrap_or_else(|e| panic!("eval `{src}` failed: {e:?}"))
}

fn extract_counter(map: &Value, key: &str) -> i64 {
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
fn run_clojure_raster_tests() {
    let test_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test");
    assert!(
        test_dir.is_dir(),
        "test dir not found: {}",
        test_dir.display()
    );

    let _mutator = cljrs_gc::register_mutator();

    let globals = {
        let runtime = cljrs_runtime::Runtime::builder()
            .execution_mode(cljrs_runtime::ExecutionMode::Tiered)
            .source_paths(vec![test_dir])
            .build()
            .expect("runtime");
        cljrs_stdlib::install(&runtime);
        runtime.into_globals()
    };

    // Register raster native functions before any Clojure code is evaluated.
    let mut registry = Registry::new(globals.clone());
    cljrs_raster::register(&mut registry);

    let mut env = Env::new(globals, "user");

    cljrs_runtime::env::callback::push_eval_context(&env);

    eval_str(&mut env, "(require 'clojure.test)");

    let mut total_pass = 0i64;
    let mut total_fail = 0i64;
    let mut total_error = 0i64;
    let mut total_test = 0i64;
    let mut failures: Vec<String> = Vec::new();

    for ns in TEST_NSES {
        eprintln!("[raster-tests] running {ns}");
        eval_str(&mut env, &format!("(require '{ns})"));
        let result = eval_str(&mut env, &format!("(clojure.test/run-tests '{ns})"));
        let pass = extract_counter(&result, "pass");
        let fail = extract_counter(&result, "fail");
        let error = extract_counter(&result, "error");
        let test = extract_counter(&result, "test");
        total_pass += pass;
        total_fail += fail;
        total_error += error;
        total_test += test;
        if fail > 0 || error > 0 {
            failures.push(format!("{ns}: {fail} fail, {error} error"));
        }
    }

    cljrs_runtime::env::callback::pop_eval_context();

    eprintln!(
        "[raster-tests] totals: {total_test} tests, {total_pass} pass, \
         {total_fail} fail, {total_error} error",
    );

    assert!(total_test > 0, "no tests ran");
    assert!(
        failures.is_empty(),
        "Clojure-side test failures:\n  {}",
        failures.join("\n  "),
    );
}
