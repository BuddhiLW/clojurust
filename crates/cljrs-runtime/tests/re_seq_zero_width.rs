//! Regression tests for zero-width matches in `re-find`/`re-seq`.
//!
//! A pattern that can match empty (`a*`, `#""`) matched at the same offset
//! forever, because the matcher resumed at the end of the previous match and an
//! empty match does not move it. `re-seq` is `take-while identity` over
//! `re-find`, and `""` is truthy, so the sequence never ended. Java's `find`
//! bumps the search position by one character after an empty match.
//!
//! The sequences are realised through `take` so that a regression fails on the
//! contents rather than hanging the suite.

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

/// Realise at most 8 elements of `expr`, so an unterminated sequence fails the
/// assertion instead of spinning.
fn assert_seq(expr: &str, expected: &str) {
    let rendered = eval_pr_str(&format!("(vec (take 8 {}))", expr));
    assert_eq!(rendered, expected, "mismatch for {}", expr);
}

#[test]
fn re_seq_terminates_on_a_pattern_that_can_match_empty() {
    assert_seq(r#"(re-seq #"a*" "aaa")"#, r#"["aaa" ""]"#);
    assert_seq(r#"(re-seq #"a*" "bab")"#, r#"["" "a" "" ""]"#);
    assert_seq(r#"(re-seq #"\d*" "a1")"#, r#"["" "1" ""]"#);
    assert_seq(r#"(re-seq #"" "ab")"#, r#"["" "" ""]"#);
    assert_seq(r#"(re-seq #"" "")"#, r#"[""]"#);
}

#[test]
fn re_seq_steps_by_characters_not_bytes() {
    assert_seq(r#"(re-seq #"x*" "é")"#, r#"["" ""]"#);
    assert_seq(r#"(re-seq #"x*" "日本")"#, r#"["" "" ""]"#);
}

#[test]
fn re_find_on_a_matcher_runs_out() {
    assert_eq!(
        eval_pr_str(r#"(let [m (re-matcher #"a*" "aaa")] [(re-find m) (re-find m) (re-find m)])"#),
        r#"["aaa" "" nil]"#
    );
}

#[test]
fn non_empty_matches_are_unchanged() {
    assert_seq(r#"(re-seq #"\d+" "a1 b22 c333")"#, r#"["1" "22" "333"]"#);
    assert_seq(r#"(re-seq #"a+" "aaa")"#, r#"["aaa"]"#);
}
