//! Map entries are a dedicated type, not plain 2-element vectors.
//!
//! `map-entry?` must be true only for real entries — the `[k v]` pairs
//! produced by seq'ing a map, `find`, or the `map-entry` constructor — and
//! false for plain vectors like `[:a 1]`. (A loose `map-entry?` broke
//! replicant.hiccup/hiccup?: 2-element hiccup such as `[:h1 "hi"]` was
//! misclassified as a map entry.)

use std::sync::Arc;

use cljrs_env::env::{Env, GlobalEnv};
use cljrs_reader::Parser;
use cljrs_value::Value;

fn make_env() -> (Arc<GlobalEnv>, Env) {
    let globals = cljrs_interp::standard_env(None, None, None);
    let env = Env::new(globals.clone(), "user");
    (globals, env)
}

fn eval_fresh(src: &str) -> Value {
    let (_, mut env) = make_env();
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_interp::eval::eval(&form, &mut env).expect("eval error");
    }
    result
}

fn assert_bool(src: &str, expected: bool) {
    assert_eq!(eval_fresh(src), Value::Bool(expected), "{src}");
}

// ── map-entry? predicate ─────────────────────────────────────────────────────

#[test]
fn plain_vectors_are_not_map_entries() {
    assert_bool("(map-entry? [:a 1])", false);
    assert_bool("(map-entry? [:h1 \"hi\"])", false);
    assert_bool("(map-entry? (vector :a 1))", false);
    assert_bool("(map-entry? [])", false);
    assert_bool("(map-entry? [1 2 3])", false);
    assert_bool("(map-entry? nil)", false);
    assert_bool("(map-entry? '(1 2))", false);
    assert_bool("(map-entry? {:a 1})", false);
}

#[test]
fn seq_of_map_yields_map_entries() {
    assert_bool("(map-entry? (first {:a 1}))", true);
    assert_bool("(map-entry? (first (seq {:a 1})))", true);
    assert_bool("(map-entry? (second (seq {:a 1 :b 2})))", true);
    assert_bool("(map-entry? (first (rest (seq {:a 1 :b 2}))))", true);
    assert_bool("(every? map-entry? (seq {:a 1 :b 2 :c 3}))", true);
    assert_bool("(map-entry? (first (hash-map :a 1)))", true);
    assert_bool("(map-entry? (first (array-map :a 1)))", true);
    assert_bool("(map-entry? (first (sorted-map :a 1)))", true);
    assert_bool("(map-entry? (first (rseq (sorted-map :a 1 :b 2))))", true);
}

#[test]
fn find_returns_map_entries() {
    assert_bool("(map-entry? (find {:a 1} :a))", true);
    assert_bool("(map-entry? (find [10 20] 1))", true);
    assert_bool("(map-entry? (find (transient {:a 1}) :a))", true);
}

#[test]
fn entries_survive_into_and_mapping() {
    // into a vector conj's the entries through unchanged.
    assert_bool("(map-entry? (first (into [] {:a 1})))", true);
    // map/filter over a map see real entries.
    assert_bool("(every? map-entry? (map identity {:a 1 :b 2}))", true);
    assert_bool("(every? map-entry? (filter (fn [e] true) {:a 1}))", true);
}

// ── map-entry constructor ────────────────────────────────────────────────────

#[test]
fn map_entry_constructor_two_args() {
    assert_bool("(map-entry? (map-entry :a 1))", true);
    assert_bool("(= (map-entry :a 1) [:a 1])", true);
    assert_bool("(= :a (key (map-entry :a 1)))", true);
    assert_bool("(= 1 (val (map-entry :a 1)))", true);
}

#[test]
fn map_entry_constructor_one_seqable_arg() {
    assert_bool("(map-entry? (map-entry [:a 1]))", true);
    assert_bool("(map-entry? (map-entry '(:a 1)))", true);
    assert_bool("(map-entry? (map-entry (map inc [1 2])))", true);
    assert_bool("(= (map-entry [:a 1]) [:a 1])", true);
}

#[test]
fn map_entry_constructor_rejects_wrong_element_count() {
    assert_bool(
        "(try (map-entry [1 2 3]) false (catch Exception e true))",
        true,
    );
    assert_bool("(try (map-entry [1]) false (catch Exception e true))", true);
}

// ── vector behavior of entries ───────────────────────────────────────────────

#[test]
fn entries_behave_like_vectors() {
    assert_bool("(vector? (first {:a 1}))", true);
    assert_bool("(sequential? (first {:a 1}))", true);
    assert_bool("(coll? (first {:a 1}))", true);
    assert_bool("(= 2 (count (first {:a 1})))", true);
    assert_bool("(= :a (nth (first {:a 1}) 0))", true);
    assert_bool("(= 1 (nth (first {:a 1}) 1))", true);
    assert_bool("(= [:a 1] (first {:a 1}))", true);
    // Destructuring works.
    assert_bool("(let [[k v] (first {:a 1})] (= [k v] [:a 1]))", true);
}

#[test]
fn derived_vectors_are_plain_vectors() {
    assert_bool("(map-entry? (conj (map-entry :a 1) 2))", false);
    assert_bool("(map-entry? (assoc (map-entry :a 1) 0 :b))", false);
    assert_bool("(map-entry? (pop (map-entry :a 1)))", false);
    assert_bool("(map-entry? (subvec (map-entry :a 1) 0 2))", false);
    assert_bool("(map-entry? (vec (map-entry :a 1)))", false);
}

// ── key/val are strict ───────────────────────────────────────────────────────

#[test]
fn key_and_val_reject_plain_vectors() {
    assert_bool("(try (key [:a 1]) false (catch Exception e true))", true);
    assert_bool("(try (val [:a 1]) false (catch Exception e true))", true);
    assert_bool("(= :a (key (first {:a 1})))", true);
    assert_bool("(= 1 (val (first {:a 1})))", true);
}
