//! The embedding chapter of the book, compiled and run.
//!
//! Every snippet in `docs/book/src/embedding/` that is meant to be real code
//! has a counterpart here, so a change to the runtime's construction, tier,
//! evaluation, or metering API breaks the build rather than silently making
//! the documentation wrong. Keep the two in step: when a test here changes,
//! change the page it mirrors.

// EvalError::Thrown wraps a full Value; the book's snippets return it as-is.
#![allow(clippy::result_large_err)]

use std::sync::Arc;

use cljrs_gc::{GcConfig, GcPtr};
use cljrs_reader::Parser;
use cljrs_runtime::env::apply::apply_value;
use cljrs_runtime::env::gas::{GasGuard, GasMeter};
use cljrs_runtime::env::gc_roots::root_value;
use cljrs_runtime::tiered::{Env, EvalError, eval};
use cljrs_runtime::{ExecutionMode, Runtime, TierState};
use cljrs_value::Value;

/// `embedding/evaluating.md` — the parse-and-evaluate loop, one allocation
/// frame per top-level form.
fn eval_str(env: &mut Env, src: &str, origin: &str) -> Result<Value, EvalError> {
    let mut parser = Parser::new(src.to_string(), origin.to_string());
    let forms = parser.parse_all().map_err(EvalError::Read)?;

    let mut result = Value::Nil;
    for form in &forms {
        let _frame = cljrs_gc::push_alloc_frame();
        result = eval(form, env)?;
    }
    Ok(result)
}

/// `embedding/index.md` — the minimal host.
#[test]
fn minimal_host() {
    let runtime = Runtime::builder()
        .execution_mode(ExecutionMode::Tiered)
        .source_paths(vec!["src".into()])
        .build()
        .expect("bootstrap clojure.core");

    cljrs_stdlib::install(&runtime);

    let mut env = runtime.env("user");
    let value = eval_str(&mut env, "(+ 1 2)", "<host>").expect("eval");
    assert_eq!(value.to_string(), "3");
}

/// `embedding/runtime-builder.md` — every builder option, and the handle.
#[test]
fn builder_options() {
    let runtime = Runtime::builder()
        .execution_mode(ExecutionMode::TreeWalk)
        .source_paths(vec!["src".into(), "resources".into()])
        .gc_config_from_env(false)
        .gc_config(Arc::new(GcConfig::with_limits(32 << 20, 64 << 20)))
        .register_gc_roots(true)
        .builtin_source("acme.rules", "(ns acme.rules) (defn allow? [_] true)")
        .eager_clojure_test(false)
        .build()
        .expect("bootstrap");

    assert_eq!(runtime.execution_mode(), ExecutionMode::TreeWalk);
    assert_eq!(runtime.tier_state(), TierState::TreeWalk);

    // An embedded source resolves through `require` with no file on disk.
    let mut env = runtime.env("user");
    eval_str(&mut env, "(require '[acme.rules :as rules])", "<host>").expect("require");
    assert_eq!(
        eval_str(&mut env, "(rules/allow? :anything)", "<host>")
            .unwrap()
            .to_string(),
        "true"
    );

    // The handle round-trips through the environment it wraps.
    let adopted = Runtime::from_globals(runtime.globals().clone());
    assert_eq!(adopted.execution_mode(), ExecutionMode::TreeWalk);
    let _globals: Arc<cljrs_runtime::env::env::GlobalEnv> = adopted.into_globals();
}

/// `embedding/execution-modes.md` — a tiered runtime lands on `Ir` without a
/// JIT backend and on `Jit` with one, and the thresholds are settable.
#[test]
fn execution_modes() {
    let no_jit = Runtime::builder()
        .execution_mode(ExecutionMode::TieredNoJit)
        .build()
        .expect("bootstrap");
    assert_eq!(no_jit.tier_state(), TierState::Ir);

    cljrs_runtime::tiered::set_ir_threshold(10);
    cljrs_runtime::tiered::set_jit_threshold(500);
    cljrs_runtime::tiered::set_osr_threshold(5_000);

    let jitted = Runtime::builder()
        .execution_mode(ExecutionMode::Tiered)
        .build()
        .expect("bootstrap");
    cljrs_compiler::jit::install(&jitted); // before any guest code runs
    cljrs_stdlib::install(&jitted);
    assert_eq!(jitted.tier_state(), TierState::Jit);
}

/// `embedding/execution-modes.md` — the depth cap of `NoGcTransaction`.
#[test]
fn depth_cap() {
    let runtime = Runtime::builder()
        .execution_mode(ExecutionMode::NoGcTransaction)
        .build()
        .expect("bootstrap");
    let mut env = runtime.env("user");
    eval_str(
        &mut env,
        "(defn down [n] (if (zero? n) 0 (down (dec n))))",
        "<t>",
    )
    .unwrap();

    let _depth = cljrs_runtime::env::depth::DepthGuard::install(8);
    let err = eval_str(&mut env, "(down 100)", "<t>").unwrap_err();
    assert!(matches!(err, EvalError::Runtime(_)), "got {err:?}");
}

/// `embedding/evaluating.md` — calling a Clojure function from Rust, and the
/// three ways of reading a `Value` back.
#[test]
fn call_clojure_from_rust() {
    let runtime = Runtime::builder().build().expect("bootstrap");
    cljrs_stdlib::install(&runtime);
    let mut env = runtime.env("user");
    eval_str(
        &mut env,
        "(defn greet [who] (str \"hello, \" who))",
        "<host>",
    )
    .unwrap();

    let f = runtime
        .globals()
        .lookup_in_ns("user", "greet")
        .expect("greet is defined");
    let _root = root_value(&f);

    let arg = Value::Str(GcPtr::new("world".to_string()));
    let out = apply_value(&f, vec![arg], &mut env).expect("call");

    // 1. Display prints readably — a string keeps its quotes.
    assert_eq!(out.to_string(), "\"hello, world\"");
    // 2. Marshalling.
    use cljrs_interop::FromValue;
    assert_eq!(String::from_value(&out).unwrap(), "hello, world");
    // 3. Matching.
    match &out {
        Value::Str(s) => assert_eq!(s.get().as_str(), "hello, world"),
        other => panic!("expected a string, got {other}"),
    }
}

/// `embedding/sandboxing.md` — a gas meter bounds a runaway loop.
#[test]
fn gas_metering() {
    let runtime = Runtime::builder().build().expect("bootstrap");
    let mut env = runtime.env("user");

    let meter = GasMeter::new(10_000);
    let guard = GasGuard::install(meter.clone());
    let result = eval_str(
        &mut env,
        "(loop [i 0] (if (< i 100000000) (recur (inc i)) i))",
        "<guest>",
    );
    drop(guard);

    assert!(
        matches!(result, Err(EvalError::GasExhausted)),
        "expected the budget to run out"
    );
}

/// `embedding/extensions.md` — extensions install into a finished runtime, and
/// a host registers its own native functions through a `Registry`.
#[test]
fn extensions_and_native_functions() {
    let runtime = Runtime::builder().build().expect("bootstrap");
    cljrs_stdlib::install(&runtime);
    let globals = runtime.globals();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();
    local.block_on(&rt, async {
        cljrs_async::init(globals);
        cljrs_io::init(globals);
        cljrs_net::init(globals);
        cljrs_charset::init(globals);
    });
    cljrs_base64::init(globals);

    let registry = cljrs_interop::Registry::new(globals.clone());
    registry.define(
        "acme.native/add",
        cljrs_interop::wrap_fn2("add", |a: i64, b: i64| Ok::<i64, String>(a + b)),
    );

    let mut env = runtime.env("user");
    assert_eq!(
        eval_str(&mut env, "(acme.native/add 3 4)", "<host>")
            .unwrap()
            .to_string(),
        "7"
    );
}
