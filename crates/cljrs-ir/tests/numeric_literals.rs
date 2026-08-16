//! Regression tests for numeric-literal lowering.
//!
//! No bignum `Const` exists yet, so integer literals are represented as `i64`.
//! A literal that fits must lower cleanly; one that overflows `i64` must surface
//! a `LowerError` rather than silently truncating to `0` (which would corrupt
//! results — see `lower_form`'s `FormKind::BigInt` arm).

use cljrs_ir::lower::{LowerError, lower_fn_body};
use cljrs_ir::{Const, Inst};
use cljrs_reader::Parser;

fn try_lower(source: &str) -> Result<cljrs_ir::IrFunction, LowerError> {
    let mut parser = Parser::new(source.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse");
    lower_fn_body(Some("test"), "user", &[], &forms, false)
}

#[test]
fn bigint_literal_within_i64_lowers_ok() {
    // `42N` lexes to a BigInt token but fits i64 — must lower without error.
    assert!(try_lower("42N").is_ok());
    // A large-but-in-range value via the overflow path is also fine.
    assert!(try_lower("9223372036854775807").is_ok()); // i64::MAX
}

#[test]
fn bigint_literal_overflowing_i64_is_an_error() {
    // Well beyond i64::MAX: previously truncated to 0, now an explicit error.
    let err = try_lower("99999999999999999999999999999999")
        .expect_err("out-of-range integer literal should not lower");
    match err {
        LowerError::UnsupportedForm(msg) => {
            assert!(
                msg.contains("out of i64 range"),
                "unexpected message: {msg}"
            );
        }
        other => panic!("expected UnsupportedForm, got {other:?}"),
    }
}

#[test]
fn exact_literal_kinds_survive_lowering() {
    let ir = try_lower(r#"1/3 1.25M #"[a-z]+""#).expect("exact literals should lower");
    let constants: Vec<&Const> = ir
        .blocks
        .iter()
        .flat_map(|block| &block.insts)
        .filter_map(|inst| match inst {
            Inst::Const(_, constant) => Some(constant),
            _ => None,
        })
        .collect();

    assert!(
        constants
            .iter()
            .any(|c| matches!(c, Const::Ratio(s) if s.as_ref() == "1/3"))
    );
    assert!(
        constants
            .iter()
            .any(|c| matches!(c, Const::BigDecimal(s) if s.as_ref() == "1.25"))
    );
    assert!(
        constants
            .iter()
            .any(|c| matches!(c, Const::Regex(s) if s.as_ref() == "[a-z]+"))
    );
}
