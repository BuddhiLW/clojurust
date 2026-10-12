//! Host features `clojure.test.check` needs to load and run on cljrs
//! (the dependency `malli.generator` pulls in). Each test pins one failure
//! seen while requiring test.check 1.1.1 from source.

use cljrs_reader::Parser;
use cljrs_runtime::env::env::Env;
use cljrs_value::Value;

fn eval_pr(src: &str) -> String {
    let globals = cljrs_runtime::Runtime::builder()
        .execution_mode(cljrs_runtime::ExecutionMode::TreeWalk)
        .build()
        .expect("runtime")
        .into_globals();
    let mut env = Env::new(globals, "user");
    let wrapped = format!("(pr-str (do {src}))");
    let mut parser = Parser::new(wrapped, "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_runtime::interp::eval::eval(&form, &mut env).expect("eval error");
    }
    match result {
        Value::Str(s) => s.get().as_str().to_string(),
        other => panic!("expected a string, got {other:?}"),
    }
}

/// clojure.test.check.rose-tree/RoseTree implements clojure.lang.Indexed.
#[test]
fn deftype_implements_indexed_for_nth() {
    assert_eq!(
        eval_pr(
            "(deftype R [root children]
               clojure.lang.Indexed
               (nth [this i] (case i 0 root 1 children))
               (nth [this i nf] (case i 0 root 1 children nf)))
             (let [r (R. 1 [2])] [(nth r 0) (nth r 1) (nth r 5 :nf) (.-root r)])"
        ),
        "[1 [2] :nf 1]"
    );
}

/// `.method` reaches an Object method a reify body defined.
#[test]
fn reify_object_method_via_interop() {
    assert_eq!(
        eval_pr("(.describe (reify Object (describe [_ x] (str \"d\" x))) 7)"),
        "\"d7\""
    );
}

/// clojure.test.check.random keeps its RNG in a ThreadLocal proxy.
#[test]
fn proxy_thread_local() {
    assert_eq!(
        eval_pr(
            "(let [n (atom 0)
                   tl (proxy [ThreadLocal] [] (initialValue [] (swap! n inc) :init))]
               [(.get tl) (.get tl) (do (.set tl :x) (.get tl)) @n])"
        ),
        "[:init :init :x 1]"
    );
}

/// clojure.test.check.generators excludes `let` and binds with `core/let`.
#[test]
fn aliased_core_special_form() {
    assert_eq!(
        eval_pr(
            "(ns foo.bar (:refer-clojure :exclude [let]) (:require [clojure.core :as core]))
             (defn f [coll] (core/let [v (vec coll)] (clojure.core/let [w v] (count w))))
             (f [1 2 3])"
        ),
        "3"
    );
}

/// clojure.test.check.generators defines its own `let` macro.
#[test]
fn fn_named_after_special_form() {
    assert_eq!(
        eval_pr(
            "(ns foo.m (:refer-clojure :exclude [let]) (:require [clojure.core :as core]))
             (defmacro let [bindings & body] `(core/let ~bindings ~@body))
             [((fn let [x] x) 1) (let [a 2] a)]"
        ),
        "[1 2]"
    );
}

/// clojure.test.check.random / generators use these JVM numeric statics.
#[test]
fn jvm_numeric_statics() {
    assert_eq!(
        eval_pr(
            "[(Long/bitCount 255) Long/MIN_VALUE (Long/reverse 1) (Math/scalb 1.5 3)
              (Math/getExponent 8.0) (Character/isDigit \\3) (Double/isNaN Double/NaN)
              (= Double/POSITIVE_INFINITY (- Double/NEGATIVE_INFINITY)) (> Double/MAX_VALUE 1e308)]"
        ),
        "[8 -9223372036854775808 -9223372036854775808 12.0 3 true true true true]"
    );
}

/// `as->` threads the name through every form (random/mix-gamma uses it inside `->`).
#[test]
fn as_thread_macro_binds_name_per_form() {
    assert_eq!(eval_pr("(as-> 5 z (inc z) (* z 2))"), "12");
    assert_eq!(eval_pr("(-> 5 (as-> z (cond-> z (> 24 z) (inc))))"), "6");
    assert_eq!(eval_pr("(as-> 5 z)"), "5");
}

/// test.check's `call-gen` destructures its Generator record: `[{h :gen} rnd size]`.
#[test]
fn map_destructuring_reads_record_fields() {
    assert_eq!(
        eval_pr("(defrecord G [gen]) (let [{f :gen :keys [gen]} (->G inc)] [(f 1) (gen 2)])"),
        "[2 3]"
    );
}

/// rose-tree's `unchunk` is `(take Double/POSITIVE_INFINITY s)`.
#[test]
fn take_infinite_count() {
    assert_eq!(eval_pr("(take Double/POSITIVE_INFINITY [1 2 3])"), "(1 2 3)");
    assert_eq!(eval_pr("(take 2.0 [1 2 3])"), "(1 2)");
}

/// `gen/double*` builds its exponent table with `reductions`.
#[test]
fn reductions_intermediate_values() {
    assert_eq!(eval_pr("(reductions + [1 2 3])"), "(1 3 6)");
    assert_eq!(eval_pr("(reductions + 10 [1 2])"), "(10 11 13)");
    assert_eq!(eval_pr("(reductions + [])"), "(0)");
    assert_eq!(eval_pr("(take 3 (reductions + (range)))"), "(0 1 3)");
}

/// `gen/elements` maps a vector over a rose tree: vectors are IFn.
#[test]
fn vector_invoked_as_fn() {
    assert_eq!(eval_pr("[([5 6] 1) (mapv [:a :b] [1 0]) (apply [7] [0])]"), "[6 [:b :a] 7]");
    assert_eq!(eval_pr("(try ([1] 3) (catch Exception _ :oob))"), ":oob");
}
