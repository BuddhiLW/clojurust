//! A call error raised inside JIT-compiled code is raised, not turned into nil.
//!
//! `rt_call` and the shared `invoke_boxed` tail used to keep only a thrown
//! value as the pending exception and return nil for every other error. So a
//! protocol method with no implementation for its argument answered nil from
//! native code while the interpreter raised "No implementation of method".
//!
//! The test warms a function up on its non-failing path until the JIT
//! publishes native code for it, then takes the failing path in native code.

use cljrs_runtime::tiered::{Env, EvalResult, eval};
use cljrs_value::Value;

#[allow(clippy::result_large_err)]
fn try_eval(env: &mut Env, src: &str) -> EvalResult {
    let mut parser = cljrs_reader::Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse");
    let mut last = Value::Nil;
    for form in &forms {
        let _alloc_frame = cljrs_gc::push_alloc_frame();
        last = eval(form, env)?;
    }
    Ok(last)
}

fn eval_str(env: &mut Env, src: &str) -> Value {
    try_eval(env, src).unwrap_or_else(|e| panic!("eval of {src:?} failed: {e:?}"))
}

/// The `ir_arity_id` of the sole arity of the fn bound to `user/<name>`.
fn arity_id_of(env: &Env, name: &str) -> u64 {
    match env.globals.lookup_in_ns("user", name) {
        Some(Value::Fn(f)) => f.get().arities[0].ir_arity_id,
        other => panic!("user/{name} is not a fn: {other:?}"),
    }
}

/// Evaluate `call` until the JIT publishes native code for `arity_id` (or
/// panic after ~15s).
fn hammer_until_native(env: &mut Env, call: &str, arity_id: u64) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while env.globals.jit().get_native_fn(arity_id).is_none() {
        assert!(
            std::time::Instant::now() < deadline,
            "JIT never published native code for {call} (arity {arity_id})"
        );
        eval_str(env, call);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[test]
fn a_missing_protocol_impl_raises_from_native_code() {
    // Tiny threshold so the worker kicks in after a handful of calls; the
    // threshold is process-wide configuration, read on every dispatch.
    cljrs_runtime::tiered::jit_state::set_jit_threshold(3);
    let _mutator = cljrs_gc::register_mutator();
    let globals = {
        let runtime = cljrs_runtime::Runtime::builder()
            .execution_mode(cljrs_runtime::ExecutionMode::Tiered)
            .build()
            .expect("runtime");
        cljrs_compiler::jit::install(&runtime);
        cljrs_stdlib::install(&runtime);
        runtime.into_globals()
    };
    let mut env = Env::new(globals.clone(), "user");
    cljrs_runtime::env::callback::push_eval_context(&env);

    eval_str(
        &mut env,
        "(defprotocol Shape (area [s]))
         (defrecord Blob [])
         (defn hot [x] (if x (area x) :ok))",
    );

    // The interpreter raises on the failing path.
    let interpreted = try_eval(&mut env, "(hot (->Blob))");
    assert!(interpreted.is_err(), "interpreter answered {interpreted:?}");

    let hot_id = arity_id_of(&env, "hot");
    hammer_until_native(&mut env, "(hot nil)", hot_id);
    assert_eq!(
        eval_str(&mut env, "(hot nil)"),
        Value::keyword(cljrs_value::Keyword::simple("ok"))
    );

    // Native code raises the same error instead of answering nil.
    let native = try_eval(&mut env, "(hot (->Blob))");
    let err = format!(
        "{:?}",
        native.expect_err("native code answered a value, not an error")
    );
    assert!(
        err.contains("No implementation of method"),
        "unexpected error: {err}"
    );

    // And it is catchable, as in the interpreter.
    assert_eq!(
        eval_str(&mut env, "(try (hot (->Blob)) (catch Exception _ :caught))"),
        Value::keyword(cljrs_value::Keyword::simple("caught"))
    );

    cljrs_runtime::env::callback::pop_eval_context();
}
