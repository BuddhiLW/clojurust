//! `recur` whose arguments contain an `await`.
//!
//! `eval_async` handles `do`/`if`/`let*`/`loop*`/`try` itself and sends every
//! other special form to the synchronous evaluator. `recur` is a special form,
//! so `(recur (conj acc (await …)) (inc i))` had its arguments evaluated by
//! sync `eval` — and sync `eval_await` parks on `FutureState::Running` with
//! `cond.wait(guard)`, blocking the single thread the `LocalSet` needs in order
//! to drive that very future. The result is a silent, permanent deadlock:
//! nothing printed, no error, just a process that never returns.
//!
//! It is the shape of every stream client — take a chunk, append it, test,
//! go again — so the obvious way to read a newline-framed reply or an HTTP
//! body to `Content-Length` hung, while the identical takes written without a
//! loop worked fine.
//!
//! ## Why the watchdog is a thread and not `tokio::time::timeout`
//!
//! A regression here does not merely fail, it wedges the machine. But the hang
//! is a *blocking* `Condvar` wait on the executor's own thread, so a
//! `tokio::time::timeout` wrapped around the future is never polled again and
//! cannot fire. The only guard that survives the failure mode is one that runs
//! on a different thread, so each case is evaluated on a spawned thread and the
//! test thread waits with `recv_timeout`. On regression the case fails in
//! `WATCHDOG` seconds instead of hanging the suite; the wedged thread is reaped
//! when the test process exits.

use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use cljrs_async::eval_async::eval_async;
use cljrs_reader::Parser;
use cljrs_runtime::env::env::{Env, GlobalEnv};

/// Generous enough that a slow machine never flakes, short enough that a real
/// deadlock is reported promptly rather than sat on.
const WATCHDOG: Duration = Duration::from_secs(20);

fn async_env() -> Arc<GlobalEnv> {
    let globals = cljrs_runtime::Runtime::builder()
        .execution_mode(cljrs_runtime::ExecutionMode::TreeWalk)
        .build()
        .expect("runtime")
        .into_globals();
    cljrs_async::init(&globals);
    globals
}

fn parse_one(src: &str) -> cljrs_reader::Form {
    Parser::new(src.to_string(), "<test>".to_string())
        .parse_all()
        .expect("parse error")
        .into_iter()
        .next()
        .expect("no form")
}

/// Evaluate `defs` synchronously, then `expr` through `eval_async`, all on a
/// spawned thread so a deadlock is observable from outside.
///
/// The value is rendered to a `String` on the worker thread: `Value` holds
/// `GcPtr`, which is `!Send`, so nothing but the rendering may cross back.
fn eval_async_on_worker(defs: &[&str], expr: &str) -> Result<String, String> {
    let defs: Vec<String> = defs.iter().map(|s| s.to_string()).collect();
    let expr = expr.to_string();
    let source = expr.clone();
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        let globals = async_env();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        let local = tokio::task::LocalSet::new();
        let outcome = local.block_on(&rt, async {
            let mut env = Env::new(globals, "user");
            for def in &defs {
                cljrs_runtime::interp::eval::eval(&parse_one(def), &mut env)
                    .map_err(|e| format!("{e:?}"))?;
            }
            eval_async(&parse_one(&source), &mut env)
                .await
                .map(|v| format!("{v}"))
                .map_err(|e| format!("{e:?}"))
        });
        // The receiver is gone on a watchdog trip; that send failing is fine.
        let _ = tx.send(outcome);
    });

    match rx.recv_timeout(WATCHDOG) {
        Ok(outcome) => outcome,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            panic!("deadlocked: {expr}\nstill blocked after {WATCHDOG:?}")
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            panic!("worker thread died evaluating: {expr}")
        }
    }
}

fn value_of(defs: &[&str], expr: &str) -> String {
    eval_async_on_worker(defs, expr).unwrap_or_else(|e| panic!("{expr}\n{e}"))
}

/// The reported bug, at its smallest: an `await` in a `recur` argument.
#[test]
fn a_recur_argument_may_await() {
    let out = value_of(
        &["(defn ^:async dbl [x] (* x 2))"],
        "(loop [acc [] i 0]
           (if (>= i 3)
             acc
             (recur (conj acc (await (dbl i))) (inc i))))",
    );
    assert_eq!(out, "[0 2 4]");
}

/// The same await reached through a `recur` that targets the enclosing
/// `^:async` fn rather than a `loop` — the other recur target, which took the
/// same synchronous path.
#[test]
fn an_async_fn_may_recur_across_an_await() {
    let out = value_of(
        &[
            "(defn ^:async dbl [x] (* x 2))",
            "(defn ^:async collect [acc i n]
               (if (>= i n)
                 acc
                 (recur (conj acc (await (dbl i))) (inc i) n)))",
        ],
        "(await (collect [] 0 3))",
    );
    assert_eq!(out, "[0 2 4]");
}

/// A `recur` arity mismatch is still reported, rather than the new async path
/// silently accepting the wrong count.
#[test]
fn recur_still_checks_its_arity() {
    let err =
        eval_async_on_worker(&[], "(loop [a 0 b 1] (recur (inc a)))").expect_err("arity mismatch");
    assert!(
        err.contains("Arity") || err.contains("recur"),
        "unhelpful error: {err}"
    );
}
