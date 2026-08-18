//! Regression tests for issue #333: `re-matches` must anchor at both ends.
//!
//! Before the fix the anchoring guard compared a capture-group count against
//! the haystack's byte length, so group-less patterns matched only one-byte
//! haystacks and a pattern whose group count happened to equal the haystack
//! length matched a strict prefix. `re-find`/`re-seq` never took that branch
//! and are covered here to keep it that way.

use std::sync::Arc;

use cljrs_reader::Parser;
use cljrs_runtime::tiered::Env;
use cljrs_value::Value;

fn make_env() -> (Arc<cljrs_runtime::env::env::GlobalEnv>, Env) {
    let globals = cljrs_runtime::Runtime::builder()
        .execution_mode(cljrs_runtime::ExecutionMode::Tiered)
        .build()
        .expect("runtime")
        .into_globals();
    let env = Env::new(globals.clone(), "user");
    (globals, env)
}

/// Evaluate `expr` and return its `pr-str` rendering.
fn eval_pr_str(expr: &str) -> String {
    let (_globals, mut env) = make_env();
    let src = format!("(pr-str {})", expr);
    let mut parser = Parser::new(src, "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_runtime::tiered::eval(&form, &mut env).expect("eval error");
    }
    match result {
        Value::Str(s) => s.get().to_string(),
        other => panic!("pr-str returned non-string: {:?}", other),
    }
}

fn assert_eval(expr: &str, expected: &str) {
    assert_eq!(eval_pr_str(expr), expected, "mismatch for {}", expr);
}

// ── whole-haystack matches ──────────────────────────────────────────────────

#[test]
fn re_matches_matches_the_whole_haystack() {
    assert_eval(r#"(re-matches #"\d+" "42")"#, r#""42""#);
    assert_eval(r#"(re-matches #"\d+" "424")"#, r#""424""#);
    assert_eval(r#"(re-matches #"\d+" "4")"#, r#""4""#);
    assert_eval(r#"(re-matches #"a+" "aaa")"#, r#""aaa""#);
    assert_eval(r#"(re-matches #".*" "hello")"#, r#""hello""#);
}

#[test]
fn re_matches_returns_groups_when_the_pattern_has_them() {
    assert_eval(
        r#"(re-matches #"(\d+)-(\d+)" "12-345")"#,
        r#"["12-345" "12" "345"]"#,
    );
}

#[test]
fn re_matches_yields_nil_for_a_non_participating_group() {
    assert_eval(r#"(re-matches #"(a)|(b)" "b")"#, r#"["b" nil "b"]"#);
}

#[test]
fn re_matches_outranks_the_engines_leftmost_first_preference() {
    // The engine's first choice for these is a strict prefix ("a", ""), which
    // an unanchored search plus a span check would reject; Clojure returns the
    // full match, so the search itself has to be anchored.
    assert_eval(r#"(re-matches #"a|ab" "ab")"#, r#""ab""#);
    assert_eval(
        r#"(re-matches #"(a|ab)(c|bc)" "abc")"#,
        r#"["abc" "a" "bc"]"#,
    );
    assert_eval(r#"(re-matches #".*?" "hello")"#, r#""hello""#);
    // …while `re-find` keeps taking that first choice.
    assert_eval(r#"(re-find #"a|ab" "ab")"#, r#""a""#);
}

#[test]
fn re_matches_keeps_group_numbering_and_inline_flags() {
    assert_eval(r#"(re-matches #"(?i)(a)(b)" "AB")"#, r#"["AB" "A" "B"]"#);
    assert_eval(r#"(re-matches #"" "")"#, r#""""#);
    assert_eval(r#"(re-matches #"" "a")"#, "nil");
}

// ── partial matches are not matches ─────────────────────────────────────────

#[test]
fn re_matches_rejects_an_unmatched_suffix() {
    // Group count 3 == length of "abc": the old guard accepted this.
    assert_eval(r#"(re-matches #"(a)(b)" "abc")"#, "nil");
    // Group count 2 == length of "ab": likewise.
    assert_eval(r#"(re-matches #"(a)" "ab")"#, "nil");
    assert_eval(r#"(re-matches #"\d+" "42x")"#, "nil");
}

#[test]
fn re_matches_rejects_an_unmatched_prefix() {
    assert_eval(r#"(re-matches #"(a)(b)" "xab")"#, "nil");
    assert_eval(r#"(re-matches #"\d+" "x42")"#, "nil");
}

#[test]
fn re_matches_rejects_a_haystack_with_no_match() {
    assert_eval(r#"(re-matches #"\d+" "abc")"#, "nil");
}

#[test]
fn re_matches_accepts_a_string_pattern() {
    assert_eval(r#"(re-matches "\\d+" "424")"#, r#""424""#);
}

// ── searching forms stay searching ──────────────────────────────────────────

#[test]
fn re_find_and_re_seq_are_unanchored() {
    assert_eval(r#"(re-find #"\d+" "42")"#, r#""42""#);
    assert_eval(r#"(re-find #"\d+" "a42b")"#, r#""42""#);
    assert_eval(r#"(re-seq #"\d+" "a1 b22 c333")"#, r#"("1" "22" "333")"#);
}
