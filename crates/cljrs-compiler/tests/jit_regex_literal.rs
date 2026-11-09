//! Tier 1 (JIT) regex literals, end to end.
//!
//! A fn whose body holds a regex literal lowers to `rt_const_regex`. The JIT
//! symbol table must resolve it, or the background worker panics with
//! "can't resolve symbol rt_const_regex" instead of publishing native code.

use cljrs_runtime::tiered::{Env, eval};
use cljrs_value::Value;

fn eval_str(env: &mut Env, src: &str) -> Value {
    let mut parser = cljrs_reader::Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse");
    let mut last = Value::Nil;
    for form in &forms {
        let _alloc_frame = cljrs_gc::push_alloc_frame();
        last = eval(form, env).unwrap_or_else(|e| panic!("eval of {src:?} failed: {e:?}"));
    }
    last
}

/// The `ir_arity_id` of the sole arity of the fn bound to `user/<name>`.
fn arity_id_of(env: &Env, name: &str) -> u64 {
    let val = env
        .globals
        .lookup_in_ns("user", name)
        .unwrap_or_else(|| panic!("user/{name} not bound"));
    match val {
        Value::Fn(f) => f.get().arities[0].ir_arity_id,
        other => panic!("user/{name} is not a fn: {other:?}"),
    }
}

#[test]
fn jit_native_code_resolves_regex_literals() {
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

    eval_str(&mut env, "(defn hot-re [] (re-find #\"a+\" \"baaac\"))");
    let id = arity_id_of(&env, "hot-re");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while env.globals.jit().get_native_fn(id).is_none() {
        assert!(
            std::time::Instant::now() < deadline,
            "JIT never published native code for hot-re (arity {id})"
        );
        assert_eq!(eval_str(&mut env, "(hot-re)"), Value::string("aaa"));
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(
        eval_str(&mut env, "(hot-re)"),
        Value::string("aaa"),
        "JIT-native code matches the regex literal"
    );

    cljrs_runtime::env::callback::pop_eval_context();
}
