//! The collection-predicate family, pinned against the JVM's answers.
//!
//! `coll?` answered false for a `defrecord` while `map?` answered true and
//! `seq` worked, which is an inconsistent trio: the value behaved as a
//! collection everywhere except the predicate that asks whether it is one.
//! That is not a cosmetic disagreement. Generic code branches on `coll?` to
//! decide whether to recurse, whether to walk, whether to break a line, so
//! every such branch silently treated a record as a scalar. It was found that
//! way: `clojure.pprint`'s layout guard is `(if (or (not (coll? x)) (fits? ...))
//! ...)`, and records therefore never broke across lines however wide they were.
//!
//! The same blind spot ran the other way for `deftype` and `reify`, which
//! answered true to `map?` and `seqable?` because every datatype instance is
//! one `TypeInstance` variant internally. On the JVM a `deftype` class
//! implements neither `IPersistentMap` nor `Seqable` unless it says so, so
//! `(map? (->T 1))` is false and `(seq (->T 1))` throws.
//!
//! A table is the right shape for this: the bug in both directions was one
//! predicate disagreeing with its neighbours about one kind of value, which is
//! visible when the answers sit next to each other and invisible when each
//! predicate has its own test.
//!
//! One environment is shared by every assertion; building a runtime
//! re-evaluates `bootstrap.cljrs`, and doing that per case is what made other
//! suites expensive (CLJRS-TEST-RUNTIME-REUSE).

use std::sync::Arc;

use cljrs_reader::Parser;
use cljrs_runtime::env::env::{Env, GlobalEnv};
use cljrs_value::Value;

fn make_env() -> (Arc<GlobalEnv>, Env) {
    let globals = cljrs_runtime::Runtime::builder()
        .execution_mode(cljrs_runtime::ExecutionMode::TreeWalk)
        .build()
        .expect("runtime")
        .into_globals();
    let env = Env::new(globals.clone(), "user");
    (globals, env)
}

fn eval_in(env: &mut Env, src: &str) -> Value {
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser
        .parse_all()
        .unwrap_or_else(|e| panic!("parse {src}: {e:?}"));
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_runtime::interp::eval::eval(&form, env)
            .unwrap_or_else(|e| panic!("eval {src}: {e:?}"));
    }
    result
}

fn printed(env: &mut Env, src: &str) -> String {
    match eval_in(env, &format!("(pr-str {src})")) {
        Value::Str(s) => s.get().to_string(),
        other => panic!("expected a string, got {other:?}"),
    }
}

/// Every predicate applied to every value, against what Clojure answers.
///
/// The expected column is the JVM's answer, read off a 1.11 REPL for the same
/// expressions. `(seq r)` on a record yields its entries, so `seqable?` is true
/// there; `(seq t)` on a `deftype` throws, so it is false.
#[test]
fn the_collection_predicates_agree_with_the_jvm_for_every_kind_of_value() {
    let (_g, mut env) = make_env();

    eval_in(
        &mut env,
        r#"
        (defrecord R [a b])
        (deftype T [a])
        (def rec (->R 1 2))
        (def typ (->T 1))
        (defprotocol IMark (mark [x]))
        (def rei (reify IMark (mark [x] :marked)))
        "#,
    );

    // An expression and the seven answers Clojure gives for it, in the order
    // the predicate list below reads them.
    type Row = (&'static str, bool, bool, bool, bool, bool, bool, bool);

    #[rustfmt::skip]
    let table: &[Row] = &[
        //  expr        coll   map    seq'l  assoc  count  seqbl  record
        ("rec",         true,  true,  false, true,  true,  true,  true),
        ("typ",         false, false, false, false, false, false, false),
        ("rei",         false, false, false, false, false, false, false),
        ("{:a 1}",      true,  true,  false, true,  true,  true,  false),
        ("[1 2]",       true,  false, true,  true,  true,  true,  false),
        ("#{1}",        true,  false, false, false, true,  true,  false),
        ("'(1 2)",      true,  false, true,  false, true,  true,  false),
        ("(seq [1 2])", true,  false, true,  false, true,  true,  false),
        ("\"ab\"",      false, false, false, false, false, true,  false),
        ("nil",         false, false, false, false, false, true,  false),
        ("1",           false, false, false, false, false, false, false),
    ];

    let mut wrong: Vec<String> = Vec::new();
    for &(expr, coll, map, sequential, associative, counted, seqable, record) in table {
        for (pred, want) in [
            ("coll?", coll),
            ("map?", map),
            ("sequential?", sequential),
            ("associative?", associative),
            ("counted?", counted),
            ("seqable?", seqable),
            ("record?", record),
        ] {
            let got = printed(&mut env, &format!("({pred} {expr})"));
            let want = if want { "true" } else { "false" };
            if got != want {
                wrong.push(format!("({pred} {expr}) => {got}, want {want}"));
            }
        }
    }

    assert!(
        wrong.is_empty(),
        "predicates disagree with Clojure:\n{}",
        wrong.join("\n")
    );
}

/// The case that started it: a record is a collection, and the rest of the
/// family says so too.
#[test]
fn a_record_is_a_collection_a_map_and_seqable_all_at_once() {
    let (_g, mut env) = make_env();

    eval_in(&mut env, "(defrecord P2 [a]) (def r (->P2 1))");

    assert_eq!(
        printed(&mut env, "[(coll? r) (map? r) (record? r) (seq r)]"),
        "[true true true ([:a 1])]"
    );
}

/// `deftype` and `reify` are not maps, so `get` on one is not a field read and
/// `seq` on one is not a walk over its fields. Pinning this is what keeps the
/// record fix from being implemented as "every TypeInstance is a collection".
#[test]
fn a_deftype_instance_is_not_a_map_and_does_not_seq() {
    let (_g, mut env) = make_env();

    eval_in(&mut env, "(deftype T2 [a]) (def t (->T2 1))");

    assert_eq!(printed(&mut env, "(coll? t)"), "false");
    assert_eq!(printed(&mut env, "(map? t)"), "false");
    assert_eq!(printed(&mut env, "(seqable? t)"), "false");
}
