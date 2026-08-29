//! Reader metadata survives from source to value.
//!
//! `^m form` is data inside `quote` and an annotation on the evaluated value
//! outside it; either way `(meta …)` must see it, including through a macro
//! that passes the annotated form along.

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

/// The error `src` fails with; panics if it succeeds.
fn eval_err(src: &str) -> String {
    let (_, mut env) = make_env();
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = match parser.parse_all() {
        Ok(forms) => forms,
        Err(e) => return format!("{e:?}"),
    };
    let mut result = Value::Nil;
    for form in forms {
        match cljrs_runtime::interp::eval::eval(&form, &mut env) {
            Ok(v) => result = v,
            Err(e) => return format!("{e:?}"),
        }
    }
    panic!("{src}\nexpected an error, got {result}")
}

#[test]
fn quote_keeps_the_annotation_as_data() {
    assert_eq!(eval_str("(meta (quote ^{:a 1} [1]))"), "{:a 1}");
    assert_eq!(eval_str("(meta '^{:a 1} [1])"), "{:a 1}");
}

#[test]
fn quoted_metadata_is_not_evaluated() {
    assert_eq!(eval_str("(meta (quote ^{:x (+ 1 2)} [1]))"), "{:x (+ 1 2)}");
}

#[test]
fn auto_keywords_inside_metadata_resolve() {
    assert_eq!(eval_str("(meta (quote ^{:x ::k} [1]))"), "{:x :user/k}");
}

#[test]
fn metadata_survives_a_macro_round_trip() {
    // The reported repro: the macro receives the annotated form as a value and
    // hands it back inside `quote`.
    assert_eq!(
        eval_str("(defmacro q [f] (list 'quote f)) (meta (q ^{:x ::k} [1]))"),
        "{:x :user/k}"
    );
}

#[test]
fn shorthand_annotations_expand() {
    assert_eq!(eval_str("(meta (quote ^:dyn sym))"), "{:dyn true}");
    assert_eq!(eval_str("(meta (quote ^String s))"), "{:tag String}");
    assert_eq!(eval_str("(meta ^:dyn [1])"), "{:dyn true}");
    assert_eq!(eval_str("(meta ^String [1])"), "{:tag String}");
}

#[test]
fn stacked_annotations_merge_with_the_outer_one_winning() {
    assert_eq!(eval_str("(meta (quote ^:a ^:b [1]))"), "{:b true, :a true}");
    assert_eq!(eval_str("(meta (quote ^{:a 1} ^{:a 2} [1]))"), "{:a 1}");
    assert_eq!(eval_str("(meta ^:a ^:b [1])"), "{:b true, :a true}");
}

#[test]
fn evaluated_annotations_see_the_enclosing_scope() {
    assert_eq!(eval_str("(let [x 5] (meta ^{:x x} [1]))"), "{:x 5}");
}

#[test]
fn the_annotated_value_is_unchanged() {
    assert_eq!(eval_str("(quote ^{:a 1} [1])"), "[1]");
    assert_eq!(eval_str("(= [1] ^{:a 1} [1])"), "true");
    assert_eq!(eval_str("(count ^{:a 1} [1 2])"), "2");
    assert_eq!(eval_str("(conj ^{:a 1} [1] 2)"), "[1 2]");
}

#[test]
fn scalars_carry_no_metadata() {
    assert_eq!(eval_str("(meta (quote ^{:a 1} 42))"), "nil");
    assert_eq!(eval_str("(meta ^{:a 1} 42)"), "nil");
    assert_eq!(eval_str("(inc ^{:a 1} 41)"), "42");
}

#[test]
fn a_def_name_tag_is_a_symbol() {
    assert_eq!(
        eval_str("(def ^String x 1) (:tag (meta (var x)))"),
        "String"
    );
}

#[test]
fn nil_metadata_leaves_no_wrapper() {
    // `->` threads with `(with-meta … (meta form))`, and `(meta form)` is nil for
    // an unannotated form. A nil annotation must carry nothing: a stored
    // nil-meta wrapper survives into `type` and breaks `identical?` on a clone.
    assert_eq!(eval_str("(meta (with-meta [1] nil))"), "nil");
    assert_eq!(eval_str("(type (with-meta {} nil))"), "Map");
    assert_eq!(
        eval_str("(let [y (with-meta {} nil)] (identical? y y))"),
        "true"
    );
    assert_eq!(eval_str("(type (-> (hash-map :a 1) (dissoc :a)))"), "Map");
    assert_eq!(
        eval_str("(let [y (-> (hash-map :a 1) (dissoc :a))] (identical? y y))"),
        "true"
    );
    assert_eq!(
        eval_str("(-> {} (with-meta {:foo 42}) (conj [:k :v]) meta)"),
        "{:foo 42}"
    );
}

#[test]
fn type_sees_through_an_annotation() {
    assert_eq!(eval_str("(type ^{:a 1} [1])"), "Vector");
    assert_eq!(eval_str("(type ^{:a 1} {})"), "Map");
    assert_eq!(eval_str("(type (with-meta '(1) {:a 1}))"), "List");
}

// ── Which forms take the annotation as runtime metadata ─────────────────────
//
// The JVM attaches an evaluated-position annotation only to a form that
// *constructs* an `IObj`. Every expected value below was measured against
// Clojure 1.12 before being pinned here.

#[test]
fn only_a_constructing_form_carries_the_annotation() {
    // Attaches: a collection literal or a function.
    assert_eq!(eval_str("(meta ^{:a 1} [1])"), "{:a 1}");
    assert_eq!(eval_str("(meta ^{:a 1} {:k 1})"), "{:a 1}");
    assert_eq!(eval_str("(meta ^{:a 1} #{1})"), "{:a 1}");
    assert_eq!(eval_str("(meta ^{:a 1} (fn [] 1))"), "{:a 1}");
    assert_eq!(eval_str("(meta ^{:a 1} #(inc %))"), "{:a 1}");

    // A hint, dropped: a call, a symbol, `quote`, `if`, `do`, `let`.
    assert_eq!(eval_str("(meta ^{:a 1} (list 1))"), "nil");
    assert_eq!(eval_str("(def x [1]) (meta ^{:a 1} x)"), "nil");
    assert_eq!(eval_str("(let [x [1]] (meta ^{:a 1} x))"), "nil");
    assert_eq!(eval_str("(meta ^{:a 1} '(1 2))"), "nil");
    assert_eq!(eval_str("(meta ^{:a 1} (if true [1] [2]))"), "nil");
    assert_eq!(eval_str("(meta ^{:a 1} (do [1]))"), "nil");
}

#[test]
fn a_dropped_annotation_is_not_even_evaluated() {
    // The annotation on a hint position must not run its side effects — the
    // JVM never evaluates one, because it never reaches the value.
    assert_eq!(
        eval_str("(def hits (atom 0)) (meta ^{:x (swap! hits inc)} (list 1)) @hits"),
        "0"
    );
    // On a constructing form it does run, exactly once.
    assert_eq!(
        eval_str("(def hits (atom 0)) (meta ^{:x (swap! hits inc)} [1]) @hits"),
        "1"
    );
}

// ── The two positions share one shorthand table ─────────────────────────────

/// `^m` inside `quote` and `^m` outside it differ only in whether the general
/// case is evaluated. Every shorthand must expand identically in both.
fn assert_shorthand_agrees(annotation: &str, expected: &str) {
    let quoted = eval_str(&format!("(meta '{annotation} [1])"));
    let evaluated = eval_str(&format!("(meta {annotation} [1])"));
    assert_eq!(
        quoted, evaluated,
        "`{annotation}` expanded differently quoted vs evaluated"
    );
    assert_eq!(quoted, expected, "`{annotation}`");
}

#[test]
fn the_shorthand_tables_cannot_drift() {
    assert_shorthand_agrees("^:dyn", "{:dyn true}");
    assert_shorthand_agrees("^Sym", "{:tag Sym}");
    assert_shorthand_agrees("^\"String\"", "{:tag \"String\"}");
    assert_shorthand_agrees("^::foo", "{:user/foo true}");
    assert_shorthand_agrees("^{:a 1}", "{:a 1}");
}

#[test]
fn a_malformed_annotation_is_rejected() {
    // The JVM reader rejects `^42 [1]` outright rather than attaching `42`.
    for src in ["(meta ^42 [1])", "(meta '^42 [1])"] {
        let err = eval_err(src);
        assert!(
            err.contains("Metadata must be Symbol, Keyword, String or Map"),
            "`{src}` was accepted (or failed for another reason): {err}"
        );
    }
}

// ── Metadata a macro attached is data, not code ─────────────────────────────

#[test]
fn a_macros_metadata_is_not_re_analysed() {
    // `value_to_form` rebuilds a `WithMeta` value as a `^meta` form. Analysing
    // that annotation again would resolve its contents as code — and the
    // annotation is not code, it is a map the macro already built.
    //
    // The inner form has to be one that *constructs*, or the annotation is
    // dropped as a hint and never analysed whatever this does: `'(vector 1)`
    // exercises the drop, `'[1]` exercises the round trip.
    assert_eq!(
        eval_str("(defmacro m [] (with-meta '(vector 1) {:tag 'String})) (m)"),
        "[1]"
    );
    assert_eq!(
        eval_str("(defmacro m [] (with-meta '[1] {:tag 'String})) (m)"),
        "[1]"
    );
    assert_eq!(
        eval_str("(defmacro m [] (with-meta '{:k 1} {:tag 'String})) (m)"),
        "{:k 1}"
    );
    // …and the annotation survives as the data it was.
    assert_eq!(
        eval_str("(defmacro m [] (with-meta '[1] {:tag 'String})) (meta (m))"),
        "{:tag String}"
    );
    // Measured divergence from Clojure 1.12: there, macro-attached metadata on
    // a constructing form is compiled as code, so `^String` resolves to the
    // *class* `java.lang.String` and `{:arglists '([x])}` fails on `x`. This
    // runtime has no classes to resolve a tag to, so it keeps both as data
    // rather than failing on the first and inventing a resolution for the
    // second.
    assert_eq!(
        eval_str("(defmacro m [] (with-meta '[1] {:arglists '([x])})) (m)"),
        "[1]"
    );
    assert_eq!(
        eval_str("(defmacro m [] (with-meta '(vector 1) {:arglists '([x])})) (m)"),
        "[1]"
    );
}

#[test]
fn the_name_mangling_macro_idiom_works() {
    // `(symbol (str "make-" (name n)))` where `n` arrived annotated.
    assert_eq!(
        eval_str(
            "(defmacro dm [n] (list 'def (symbol (str \"make-\" (name n))) 1)) \
             (dm ^:private thing) make-thing"
        ),
        "1"
    );
}

// ── Accessors and predicates see through an annotation ──────────────────────

// Keeping `Value::Symbol` in `supports_meta` (the quoted-path tests depend on
// it) makes metadata-wrapped symbols routine, so every accessor and predicate
// has to look through the wrapper. `'^{:a 1} x` is the reader's way of
// producing one; `with-meta` is the programmatic way.

#[test]
fn the_named_accessors_are_metadata_transparent() {
    assert_eq!(eval_str("(name (first '[^{:a 1} x]))"), "\"x\"");
    assert_eq!(eval_str("(name (with-meta 'a/x {:a 1}))"), "\"x\"");
    assert_eq!(eval_str("(namespace (with-meta 'a/x {:a 1}))"), "\"a\"");
    assert_eq!(eval_str("(namespace (with-meta 'x {:a 1}))"), "nil");
    assert_eq!(eval_str("(name (with-meta :k {:a 1}))"), "\"k\"");
    assert_eq!(eval_str("(name (with-meta \"s\" {:a 1}))"), "\"s\"");
}

#[test]
fn type_predicates_are_metadata_transparent() {
    assert_eq!(eval_str("(symbol? (first '[^{:a 1} x]))"), "true");
    assert_eq!(eval_str("(symbol? (with-meta 'x {:a 1}))"), "true");
    assert_eq!(eval_str("(seq? (with-meta '(1 2) {:a 1}))"), "true");
    assert_eq!(eval_str("(fn? ^{:a 1} (fn [] 1))"), "true");
    assert_eq!(eval_str("(ifn? ^{:a 1} (fn [] 1))"), "true");
    assert_eq!(eval_str("(vector? ^{:a 1} [1])"), "true");
    assert_eq!(eval_str("(map? ^{:a 1} {})"), "true");
    assert_eq!(eval_str("(set? ^{:a 1} #{1})"), "true");
    assert_eq!(eval_str("(coll? ^{:a 1} [1])"), "true");
    assert_eq!(eval_str("(string? (with-meta \"s\" {:a 1}))"), "true");
    assert_eq!(eval_str("(keyword? (with-meta :k {:a 1}))"), "true");
    assert_eq!(eval_str("(number? (with-meta 1 {:a 1}))"), "true");
    assert_eq!(eval_str("(nil? (with-meta nil {:a 1}))"), "true");
}

// ── Syntax-quote ────────────────────────────────────────────────────────────

#[test]
fn syntax_quote_processes_an_annotated_form() {
    // Falling through to `form_to_value` here skipped auto-gensym resolution,
    // so this expanded to an unresolvable `y__auto__`.
    assert_eq!(eval_str("(defmacro g [] `(let [^long y# 1] y#)) (g)"), "1");
    // …and the annotation still lands on the value, as it does inside `quote`.
    assert_eq!(eval_str("(meta `^{:a 1} [1])"), "{:a 1}");
    assert_eq!(eval_str("(meta `^:a x)"), "{:a true}");
    assert_eq!(eval_str("(let [z 7] (meta `^{:x ~z} [1]))"), "{:x 7}");
}
