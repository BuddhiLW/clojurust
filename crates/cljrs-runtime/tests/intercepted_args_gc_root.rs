//! A collection that lands inside an intercepted native must not trace the
//! argument Vec the native was handed.
//!
//! `eval_call` evaluates an intercepted native's arguments into a Vec, roots
//! that Vec on the value shadow stack and then MOVES it into the native.
//! `alter-meta!` takes its arguments apart and hands the rest to
//! `Vec::extend`, which frees the buffer. It then calls `f` through
//! `apply_value`, whose GC safepoint collects, and the root into the moved Vec
//! points at freed memory. That was the SIGSEGV/SIGTRAP in the AOT
//! conformance suite after `clojure.core-test.qualified-keyword-qmark`: every
//! `deftest` expands to `alter-meta!`, and 7498e85 only changed where a
//! collection happened to fall.
//!
//! Each test forces the collection into exactly that window: an argument of
//! `alter-meta!` requests a GC, and the next safepoint is the one in
//! `eval_alter_meta`'s `apply_value`. The wide case makes the argument Vec
//! larger than glibc's mmap threshold, so freeing it unmaps the pages and
//! tracing the dangling root faults every time instead of reading stale but
//! mapped bits.

#![cfg(not(feature = "no-gc"))]

mod common;

use std::fmt::Write as _;
use std::sync::Arc;

use cljrs_value::Value;

fn install_request_gc(globals: &cljrs_runtime::env::env::GlobalEnv, ns: &str) {
    let nf = cljrs_value::NativeFn {
        name: Arc::from("request-gc!"),
        arity: cljrs_value::Arity::Fixed(1),
        func: Arc::new(|args: &[Value]| {
            cljrs_gc::request_gc();
            Ok(args[0].clone())
        }),
    };
    globals.intern(
        ns,
        Arc::from("request-gc!"),
        Value::NativeFunction(cljrs_gc::GcPtr::new(nf)),
    );
}

#[test]
fn alter_meta_survives_a_collection_in_its_own_call() {
    let (globals, mut env) = common::fresh_env();
    let ns = env.current_ns.to_string();
    install_request_gc(&globals, &ns);

    let src = r#"
      (def target 0)
      (dotimes [i 500]
        (alter-meta! (var target) assoc
                     :n i :s (str "s" i) :v [i (str i)] :m (request-gc! {:i i})))
      (meta (var target))
    "#;
    common::eval_in(&mut env, src).expect("eval");
    assert_eq!(
        common::eval_in(&mut env, "(:n (meta (var target)))").unwrap(),
        Value::Long(499)
    );
    assert_eq!(
        common::eval_in(&mut env, "(:v (meta (var target)))").unwrap(),
        common::eval_in(&mut env, "[499 \"499\"]").unwrap()
    );
}

#[test]
fn alter_meta_with_an_argument_vec_freed_back_to_the_os() {
    let (globals, mut env) = common::fresh_env();
    let ns = env.current_ns.to_string();
    install_request_gc(&globals, &ns);

    // Over 256 KiB of argument slots: far past the 128 KiB mmap threshold.
    let pairs = (256 * 1024 / std::mem::size_of::<Value>()) / 2 + 1;
    let mut call = String::from("(alter-meta! (var target) assoc");
    for i in 0..pairs - 1 {
        write!(call, " :k{i} (str {i})").unwrap();
    }
    let i = pairs - 1;
    write!(call, " :k{i} (request-gc! (str {i})))").unwrap();

    common::eval_in(&mut env, "(def target 0)").expect("def");
    common::eval_in(&mut env, &call).expect("alter-meta!");
    let last = format!("(get (meta (var target)) :k{})", pairs - 1);
    assert_eq!(
        common::eval_in(&mut env, &last).unwrap(),
        Value::string(format!("{}", pairs - 1))
    );
    assert_eq!(
        common::eval_in(&mut env, "(count (meta (var target)))").unwrap(),
        Value::Long(pairs as i64)
    );
}
