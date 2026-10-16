//! `type_tag_matches` must agree exactly with `type_tag_of`.
//!
//! They are two spellings of one table: `type_tag_of` allocates an `Arc<str>`,
//! `type_tag_matches` compares without allocating so an inline cache can check
//! a cached dispatch tag on the hot path. Their doc comments and the crate
//! README both promise they agree — nothing enforced it, and they drifted the
//! moment `type_tag_of` learned to unwrap a metadata wrapper: the miss path
//! then cached `"Vector"` while the fast path re-derived `"Object"`, so every
//! annotated dispatch value became a permanent cache miss that re-resolved and
//! rewrote the entry under lock on each call.
//!
//! Silent, and only a slowdown — which is why it needs a test rather than a
//! bug report.

use std::sync::Arc;

use cljrs_reader::Parser;
use cljrs_runtime::env::apply::{type_tag_matches, type_tag_of};
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

fn eval_value(env: &mut Env, src: &str) -> Value {
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_runtime::interp::eval::eval(&form, env)
            .unwrap_or_else(|e| panic!("{src}\neval: {e:?}"));
    }
    result
}

/// One expression per `type_tag_of` arm that a Clojure program can produce.
const VALUES: &[&str] = &[
    "nil",
    "true",
    "1",
    "1.5",
    "1N",
    "1.5M",
    "1/2",
    "\\c",
    "\"s\"",
    ":kw",
    "'sym",
    "'(1 2)",
    "[1 2]",
    "{:k 1}",
    "#{1}",
    "(fn [] 1)",
    "(atom 1)",
    "(var +)",
    "(volatile! 1)",
    "(delay 1)",
    "(promise)",
    "(lazy-seq [1])",
    "(cons 1 '(2))",
];

#[test]
fn the_two_tag_tables_agree_on_every_value() {
    let (_g, mut env) = make_env();
    for src in VALUES {
        let v = eval_value(&mut env, src);
        let tag = type_tag_of(&v);
        assert!(
            type_tag_matches(&v, &tag),
            "`{src}` has tag `{tag}` but type_tag_matches denies it"
        );
    }
}

#[test]
fn the_two_tag_tables_agree_through_a_metadata_wrapper() {
    let (_g, mut env) = make_env();
    for src in VALUES {
        let bare = eval_value(&mut env, src);
        let bare_tag = type_tag_of(&bare);

        let annotated = eval_value(&mut env, &format!("(with-meta {src} {{:probe 1}})"));
        let annotated_tag = type_tag_of(&annotated);

        assert_eq!(
            &*annotated_tag, &*bare_tag,
            "`{src}` changed its dispatch tag under an annotation"
        );
        assert!(
            type_tag_matches(&annotated, &annotated_tag),
            "annotated `{src}` has tag `{annotated_tag}` but type_tag_matches denies it — \
             every dispatch on it is a permanent inline-cache miss"
        );
        assert!(
            type_tag_matches(&annotated, &bare_tag),
            "annotated `{src}` does not match the bare tag `{bare_tag}`"
        );
    }
}

#[test]
fn a_wrong_tag_is_still_rejected() {
    // The positive control: `type_tag_matches` must not have become
    // "always true", which would satisfy every assertion above.
    let (_g, mut env) = make_env();
    for src in VALUES {
        let v = eval_value(&mut env, src);
        assert!(
            !type_tag_matches(&v, "NoSuchTag"),
            "`{src}` matched a tag that names nothing"
        );
    }
}

fn try_eval(env: &mut Env, src: &str) -> Result<Value, String> {
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_runtime::interp::eval::eval(&form, env).map_err(|e| format!("{e:?}"))?;
    }
    Ok(result)
}

/// The `Value::TypeInstance` arm is the one an inline cache exercises hardest:
/// it is the protocol-dispatch path. Its instances need a definition first, so
/// they cannot sit in `VALUES`; each entry is (definition, instance, tag).
const DATATYPES: &[(&str, &str, &str)] = &[
    ("(deftype Point [x y])", "(->Point 1 2)", "Point"),
    ("(defrecord Pair [a b])", "(->Pair 1 2)", "Pair"),
    ("(defrecord Pair [a b])", "(map->Pair {:a 1 :b 2})", "Pair"),
];

#[test]
fn the_two_tag_tables_agree_on_every_datatype_instance() {
    let (_g, mut env) = make_env();
    for (definition, instance, expected) in DATATYPES {
        eval_value(&mut env, definition);
        let v = eval_value(&mut env, instance);
        let tag = type_tag_of(&v);
        assert_eq!(&*tag, *expected, "`{instance}` carries the wrong tag");
        assert!(
            type_tag_matches(&v, &tag),
            "`{instance}` has tag `{tag}` but type_tag_matches denies it"
        );
        assert!(
            !type_tag_matches(&v, "NoSuchTag"),
            "`{instance}` matched a tag that names nothing"
        );
    }
}

#[test]
fn a_record_keeps_its_tag_through_a_metadata_wrapper() {
    // A record is the datatype that accepts metadata, so it is the datatype
    // through which the wrapper case of the `TypeInstance` arm is reachable.
    let (_g, mut env) = make_env();
    eval_value(&mut env, "(defrecord Pair [a b])");
    let bare = eval_value(&mut env, "(->Pair 1 2)");
    let annotated = eval_value(&mut env, "(with-meta (->Pair 1 2) {:probe 1})");
    assert_eq!(
        &*type_tag_of(&annotated),
        &*type_tag_of(&bare),
        "a record changed its dispatch tag under an annotation"
    );
    assert!(
        type_tag_matches(&annotated, "Pair"),
        "an annotated record is a permanent inline-cache miss"
    );
    assert!(!type_tag_matches(&annotated, "NoSuchTag"));
}

#[test]
fn a_deftype_instance_refuses_a_metadata_wrapper() {
    // The negative half of the test above: a deftype is not IObj on the JVM,
    // and cljrs refuses `with-meta` on one, so the wrapper case of the arm is
    // unreachable for it by construction rather than merely untested.
    let (_g, mut env) = make_env();
    eval_value(&mut env, "(deftype Point [x y])");
    assert!(
        try_eval(&mut env, "(with-meta (->Point 1 2) {:probe 1})").is_err(),
        "with-meta on a deftype instance must be refused"
    );
}
