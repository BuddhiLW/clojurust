//! Every intercepted native must work on **both** dispatch paths.
//!
//! `apply`, `swap!`, `resolve`, `intern` and the whole `ns-*` family need the
//! environment, so their entries in the builtin table are sentinel stubs that
//! error when invoked directly. The tree-walker intercepts them in `eval_call`;
//! the tier-1 IR interpreter has to intercept the same set. It used to carry
//! its own shorter list, so a function that called `(resolve 'map)` worked
//! until it got hot and then failed with "resolve sentinel should not be called
//! directly" — cold-correct, hot-broken, and invisible to any tree-walking
//! test.
//!
//! Two guards here. `intercept_table_is_total` is structural: it fails if a
//! name is added to the enumeration without a dispatch arm. The rest are
//! behavioural: each drives a real call through the IR interpreter, which is
//! the path the shipped `cljrs` binary takes (`ExecutionMode::Tiered` is the
//! default). This file is its own binary because eager lowering is
//! process-wide.

use std::sync::Arc;

use cljrs_reader::Parser;
use cljrs_runtime::env::env::{Env, GlobalEnv};
use cljrs_runtime::interp::apply::is_form_intercepted;
use cljrs_value::Value;

/// One runtime for the whole file: bootstrapping `clojure.core` per assertion
/// is the dominant cost of a test like this.
fn make_env() -> (Arc<GlobalEnv>, Env) {
    // Process-wide, and the reason this test is its own binary.
    cljrs_runtime::tiered::force_eager_lowering();
    let globals = cljrs_runtime::Runtime::builder()
        .execution_mode(cljrs_runtime::ExecutionMode::TieredNoJit)
        .build()
        .expect("runtime")
        .into_globals();
    let env = Env::new(globals.clone(), "user");
    (globals, env)
}

fn eval_in(env: &mut Env, src: &str) -> Result<Value, String> {
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().map_err(|e| format!("parse: {e:?}"))?;
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_runtime::interp::eval::eval(&form, env).map_err(|e| format!("{e}"))?;
    }
    Ok(result)
}

/// Structural guard: every sentinel builtin is actually intercepted.
///
/// `intercepted_native` is now the single definition — `is_form_intercepted`
/// and `dispatch_intercepted` are projections of it, so those three cannot
/// disagree by construction and need no test.
///
/// What remains is the one copy that cannot be deleted: `builtins.rs` has to
/// register a name for it to exist as a var at all, and it registers the
/// intercepted ones as sentinel stubs that error when called. That registration
/// carries real per-name information (arity, docstring), so it is not pure
/// duplication — but its SET membership is, and a name registered as a sentinel
/// without a dispatch arm is exactly the latent defect this file exists for.
/// So the sets are asserted equal, against the source.
#[test]
fn every_sentinel_builtin_is_intercepted() {
    let src = include_str!("../src/builtins/builtins.rs");

    // Rows look like `("ns-publics", Arity::Fixed(1), builtin_ns_publics_sentinel),`
    // and sometimes wrap across four lines, so this pairs each single-token
    // quoted name with the next `builtin_*_sentinel` handler that follows it.
    // A `fn` line resets the pairing so the stub DEFINITIONS are not read as
    // registrations, and a quoted token containing a space is prose, not a name.
    let mut registered: Vec<String> = Vec::new();
    let mut pending: Option<&str> = None;
    for line in src.lines() {
        let line = line.trim_start();
        if line.starts_with("fn ") || line.starts_with("pub fn ") {
            pending = None;
            continue;
        }
        if let Some(open) = line.find('"')
            && let Some(len) = line[open + 1..].find('"')
        {
            let token = &line[open + 1..open + 1 + len];
            if !token.is_empty() && !token.contains(' ') {
                pending = Some(token);
            }
        }
        if line.contains("builtin_")
            && line.contains("_sentinel")
            && let Some(name) = pending.take()
        {
            registered.push(name.to_string());
        }
    }

    assert!(
        registered.len() > 20,
        "the scraper found only {} sentinel rows, so it has stopped matching the \
         table's shape and is no longer testing anything",
        registered.len()
    );

    let orphans: Vec<&String> = registered
        .iter()
        .filter(|name| !is_form_intercepted(name))
        .collect();
    assert!(
        orphans.is_empty(),
        "registered as sentinel builtins but not intercepted, so calling them \
         raises \"sentinel should not be called directly\": {orphans:?}"
    );
}

/// Behavioural guard: the names the IR interpreter used to miss.
///
/// Each is called through a function, which eager lowering has already turned
/// into IR — so this is the tiered path, not the tree-walker.
#[test]
fn intercepted_natives_survive_ir_promotion() {
    let (_globals, mut env) = make_env();

    let cases: &[(&str, &str, &str)] = &[
        (
            "resolve",
            "(defn f [] (boolean (resolve 'map))) (f)",
            "true",
        ),
        (
            "ns-resolve",
            "(defn f [] (boolean (ns-resolve 'clojure.core 'map))) (f)",
            "true",
        ),
        (
            "ns-publics",
            "(defn f [] (map? (ns-publics 'clojure.core))) (f)",
            "true",
        ),
        (
            "ns-interns",
            "(defn f [] (map? (ns-interns 'clojure.core))) (f)",
            "true",
        ),
        (
            "ns-refers",
            "(defn f [] (map? (ns-refers 'clojure.core))) (f)",
            "true",
        ),
        (
            "ns-map",
            "(defn f [] (map? (ns-map 'clojure.core))) (f)",
            "true",
        ),
        (
            "ns-aliases",
            "(defn f [] (map? (ns-aliases 'clojure.core))) (f)",
            "true",
        ),
        (
            "find-ns",
            "(defn f [] (boolean (find-ns 'clojure.core))) (f)",
            "true",
        ),
        (
            "the-ns",
            "(defn f [] (boolean (the-ns 'clojure.core))) (f)",
            "true",
        ),
        ("all-ns", "(defn f [] (pos? (count (all-ns)))) (f)", "true"),
        (
            "create-ns",
            "(defn f [] (boolean (create-ns 'probe.tiered))) (f)",
            "true",
        ),
        (
            "intern",
            "(defn f [] (do (intern (create-ns 'probe.tiered) 'answer 42) \
             (deref (ns-resolve 'probe.tiered 'answer)))) (f)",
            "42",
        ),
        (
            "alter-meta!",
            "(defn target [] nil) \
             (defn f [] (do (alter-meta! #'target assoc :tag :marked) (:tag (meta #'target)))) (f)",
            ":marked",
        ),
        (
            "remove-ns",
            "(defn f [] (do (create-ns 'probe.doomed) (remove-ns 'probe.doomed) \
             (nil? (find-ns 'probe.doomed)))) (f)",
            "true",
        ),
        // `apply` as a *value* cannot lower to `KnownFn::Apply`; it reaches the
        // interpreter as the sentinel NativeFunction.
        (
            "apply-as-value",
            "(defn f [] ((first [apply]) + [1 2 3])) (f)",
            "6",
        ),
        (
            "swap!-as-value",
            "(defn f [] (let [a (atom 0) g (first [swap!])] (g a inc) (deref a))) (f)",
            "1",
        ),
        (
            "atom-as-value",
            "(defn f [] (deref ((first [atom]) :v))) (f)",
            ":v",
        ),
        (
            "bound-fn*",
            "(defn f [] ((bound-fn* (fn [] :bound)))) (f)",
            ":bound",
        ),
    ];

    let mut failures = Vec::new();
    for (label, src, expected) in cases {
        match eval_in(&mut env, src) {
            Ok(v) => {
                let got = format!("{v}");
                if got != *expected {
                    failures.push(format!("{label}: expected {expected}, got {got}"));
                }
            }
            Err(e) => failures.push(format!("{label}: {e}")),
        }
    }
    assert!(
        failures.is_empty(),
        "on the IR path:\n  {}",
        failures.join("\n  ")
    );
}
