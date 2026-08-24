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

// The `Value::TypeInstance` arm — the one an inline cache exercises hardest —
// needs `deftype`, which this branch's base does not have. It is covered where
// `deftype` lands.
