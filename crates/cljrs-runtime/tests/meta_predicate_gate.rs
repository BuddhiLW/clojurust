//! Every `clojure.core` predicate looks through a metadata wrapper.
//!
//! Patching the predicates one at a time is how the divergence arose: `map?`
//! and `vector?` unwrapped, `symbol?`, `seq?` and `fn?` did not, and nothing
//! said which group a newly added predicate belonged in. This gate is driven by
//! `ns-publics` rather than by a hand-written list, so a predicate added later
//! is checked without anyone remembering to add it here.
//!
//! [`the_gate_can_fail`] is the other half: it points the same comparison at
//! `meta`, which is legitimately *not* transparent. A gate that cannot flag it
//! would be reporting "no offenders" about its own inability to look.

use std::sync::Arc;

use cljrs_reader::Parser;
use cljrs_runtime::env::env::{Env, GlobalEnv};
use cljrs_value::Value;

fn make_env() -> (Arc<GlobalEnv>, Env) {
    let globals = cljrs_runtime::Runtime::builder()
        .execution_mode(cljrs_runtime::ExecutionMode::TreeWalk)
        .eager_clojure_test(true)
        .build()
        .expect("runtime")
        .into_globals();
    let env = Env::new(globals.clone(), "user");
    (globals, env)
}

fn eval_str(src: &str) -> String {
    let (_, mut env) = make_env();
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_runtime::interp::eval::eval(&form, &mut env)
            .unwrap_or_else(|e| panic!("{src}\neval: {e:?}"));
    }
    format!("{result}")
}

/// Values that can carry metadata, one per `IObj` shape a predicate might see.
const PRELUDE: &str = r#"
(def samples [[1] {:k 1} #{1} '(1 2) 'sym (fn [] 1)])
(defn attempt [f v]
  (try [:ok (pr-str (f v))] (catch Throwable e [:err (str (ex-message e))])))
(defn disagreements [f]
  (filter some?
    (map (fn [s]
           (let [bare (attempt f s)
                 ann  (attempt f (with-meta s {:probe 1}))]
             (when (not= bare ann) [(pr-str s) bare ann])))
         samples)))
(defn predicate-vars []
  (filter (fn [p] (and (= \? (last (str (first p))))
                       (fn? (deref (second p)))))
          (sort-by (comp str first) (seq (ns-publics 'clojure.core)))))
"#;

#[test]
fn every_core_predicate_is_metadata_transparent() {
    let offenders = format!(
        "{PRELUDE}
         (def offenders
           (mapcat (fn [p]
                     (map (fn [d] (cons (str (first p)) d))
                          (disagreements (deref (second p)))))
                   (predicate-vars)))"
    );
    let count = eval_str(&format!("{offenders} (count offenders)"));
    if count != "0" {
        let listed = eval_str(&format!("{offenders} (pr-str (vec (take 20 offenders)))"));
        panic!("{count} predicate/value pairs answer differently for an annotated value: {listed}");
    }
}

#[test]
fn the_gate_actually_inspects_something() {
    // Guards against the gate passing because it found no predicates to check
    // (a wrong `ns-publics`, an empty sample list, a broken `last`).
    let checked = eval_str(&format!("{PRELUDE} (count (predicate-vars))"));
    let checked: usize = checked.parse().expect("count");
    assert!(
        checked >= 50,
        "only {checked} predicates were inspected — the gate is not seeing core"
    );

    let samples = eval_str(&format!("{PRELUDE} (count samples)"));
    assert_eq!(samples, "6");
}

#[test]
fn the_gate_can_fail() {
    // `meta` is the positive control: it *must* answer differently for an
    // annotated value, so the comparison the gate relies on is proven live.
    let flagged = eval_str(&format!("{PRELUDE} (count (disagreements meta))"));
    assert_eq!(
        flagged, "6",
        "the control was not flagged — the gate cannot detect a difference"
    );
}
