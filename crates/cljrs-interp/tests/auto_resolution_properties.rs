//! Property tests for `resolve_auto_forms`.
//!
//! The law is totality: no `AutoKeyword` / `AutoSymbol` survives anywhere in
//! the tree, at any nesting, under any wrapper. `unresolved` below matches
//! `FormKind` exhaustively with no wildcard arm, so a new variant that can hold
//! a form breaks this test at compile time rather than silently going unwalked.

use std::sync::Arc;

use cljrs_builtins::form::resolve_auto_forms;
use cljrs_env::env::Env;
use cljrs_reader::form::{Form, FormKind};
use cljrs_types::span::Span;
use proptest::prelude::*;
use proptest::test_runner::TestRunner;

fn span() -> Span {
    Span::new(Arc::new("<proptest>".to_string()), 0, 0, 1, 1)
}

fn form(kind: FormKind) -> Form {
    Form::new(kind, span())
}

fn bare_name() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9]{0,5}"
}

/// A form tree over every `FormKind` that can contain another form, with
/// auto-resolved identifiers reachable at any depth.
fn any_form() -> impl Strategy<Value = Form> {
    let leaf = prop_oneof![
        bare_name().prop_map(|s| form(FormKind::AutoKeyword(s))),
        bare_name().prop_map(|s| form(FormKind::AutoSymbol(s))),
        bare_name().prop_map(|s| form(FormKind::Symbol(s))),
        bare_name().prop_map(|s| form(FormKind::Keyword(s))),
        any::<i64>().prop_map(|i| form(FormKind::Int(i))),
        Just(form(FormKind::Nil)),
    ];
    leaf.prop_recursive(4, 48, 3, |inner| {
        let seqs = prop_oneof![
            prop::collection::vec(inner.clone(), 0..3).prop_map(|v| form(FormKind::List(v))),
            prop::collection::vec(inner.clone(), 0..3).prop_map(|v| form(FormKind::Vector(v))),
            prop::collection::vec(inner.clone(), 0..3).prop_map(|v| form(FormKind::Set(v))),
            prop::collection::vec(inner.clone(), 0..3).prop_map(|v| form(FormKind::AnonFn(v))),
            prop::collection::vec(inner.clone(), 0..2).prop_map(|v| {
                let pairs = v.iter().cloned().chain(v.iter().cloned()).collect();
                form(FormKind::Map(pairs))
            }),
            prop::collection::vec(inner.clone(), 0..3).prop_map(|clauses| form(
                FormKind::ReaderCond {
                    splicing: false,
                    clauses
                }
            )),
        ];
        let wrappers = prop_oneof![
            inner
                .clone()
                .prop_map(|f| form(FormKind::Quote(Box::new(f)))),
            inner
                .clone()
                .prop_map(|f| form(FormKind::SyntaxQuote(Box::new(f)))),
            inner
                .clone()
                .prop_map(|f| form(FormKind::Unquote(Box::new(f)))),
            inner
                .clone()
                .prop_map(|f| form(FormKind::UnquoteSplice(Box::new(f)))),
            inner
                .clone()
                .prop_map(|f| form(FormKind::Deref(Box::new(f)))),
            inner.clone().prop_map(|f| form(FormKind::Var(Box::new(f)))),
            (inner.clone(), inner.clone())
                .prop_map(|(m, f)| form(FormKind::Meta(Box::new(m), Box::new(f)))),
            (bare_name(), inner)
                .prop_map(|(tag, f)| form(FormKind::TaggedLiteral(tag, Box::new(f)))),
        ];
        prop_oneof![seqs, wrappers]
    })
}

/// How many auto-resolved identifiers the tree still holds.
fn unresolved(f: &Form) -> usize {
    let seq = |forms: &[Form]| forms.iter().map(unresolved).sum::<usize>();
    match &f.kind {
        FormKind::AutoKeyword(_) | FormKind::AutoSymbol(_) => 1,

        FormKind::List(v)
        | FormKind::Vector(v)
        | FormKind::Map(v)
        | FormKind::Set(v)
        | FormKind::AnonFn(v) => seq(v),
        FormKind::ReaderCond { clauses, .. } => seq(clauses),

        FormKind::Quote(inner)
        | FormKind::SyntaxQuote(inner)
        | FormKind::Unquote(inner)
        | FormKind::UnquoteSplice(inner)
        | FormKind::Deref(inner)
        | FormKind::Var(inner)
        | FormKind::TaggedLiteral(_, inner) => unresolved(inner),
        FormKind::Meta(meta, inner) => unresolved(meta) + unresolved(inner),

        FormKind::Nil
        | FormKind::Bool(_)
        | FormKind::Int(_)
        | FormKind::BigInt(_)
        | FormKind::Float(_)
        | FormKind::BigDecimal(_)
        | FormKind::Ratio(_)
        | FormKind::Char(_)
        | FormKind::Str(_)
        | FormKind::Regex(_)
        | FormKind::Symbolic(_)
        | FormKind::Symbol(_)
        | FormKind::Keyword(_) => 0,
    }
}

/// Total node count, so a walk cannot "resolve" by dropping subtrees.
fn nodes(f: &Form) -> usize {
    let seq = |forms: &[Form]| forms.iter().map(nodes).sum::<usize>();
    1 + match &f.kind {
        FormKind::List(v)
        | FormKind::Vector(v)
        | FormKind::Map(v)
        | FormKind::Set(v)
        | FormKind::AnonFn(v) => seq(v),
        FormKind::ReaderCond { clauses, .. } => seq(clauses),
        FormKind::Quote(inner)
        | FormKind::SyntaxQuote(inner)
        | FormKind::Unquote(inner)
        | FormKind::UnquoteSplice(inner)
        | FormKind::Deref(inner)
        | FormKind::Var(inner)
        | FormKind::TaggedLiteral(_, inner) => nodes(inner),
        FormKind::Meta(meta, inner) => nodes(meta) + nodes(inner),
        _ => 0,
    }
}

fn run(check: impl Fn(&Form, &Env) -> Result<(), TestCaseError>) {
    let globals = cljrs_interp::standard_env(None, None, None);
    let env = Env::new(globals, "my.app");
    TestRunner::default()
        .run(&any_form(), |f| check(&f, &env))
        .expect("property failed");
}

#[test]
fn no_auto_identifier_survives_resolution() {
    run(|f, env| {
        let resolved = resolve_auto_forms(f, env).expect("bare names always resolve");
        prop_assert_eq!(unresolved(&resolved), 0);
        Ok(())
    });
}

#[test]
fn resolution_preserves_the_shape_of_the_tree() {
    run(|f, env| {
        let resolved = resolve_auto_forms(f, env).expect("bare names always resolve");
        prop_assert_eq!(nodes(&resolved), nodes(f));
        Ok(())
    });
}

#[test]
fn resolution_is_idempotent() {
    run(|f, env| {
        let once = resolve_auto_forms(f, env).expect("bare names always resolve");
        let twice = resolve_auto_forms(&once, env).expect("already resolved");
        prop_assert_eq!(twice, once);
        Ok(())
    });
}

#[test]
fn every_resolved_identifier_carries_the_current_namespace() {
    run(|f, env| {
        let resolved = resolve_auto_forms(f, env).expect("bare names always resolve");
        prop_assert_eq!(qualified_under(&resolved, "my.app/"), auto_count(f));
        Ok(())
    });
}

/// Identifiers whose name begins with `prefix`, counted over the whole tree.
fn qualified_under(f: &Form, prefix: &str) -> usize {
    let seq = |forms: &[Form]| {
        forms
            .iter()
            .map(|x| qualified_under(x, prefix))
            .sum::<usize>()
    };
    match &f.kind {
        FormKind::Keyword(s) | FormKind::Symbol(s) => usize::from(s.starts_with(prefix)),
        FormKind::List(v)
        | FormKind::Vector(v)
        | FormKind::Map(v)
        | FormKind::Set(v)
        | FormKind::AnonFn(v) => seq(v),
        FormKind::ReaderCond { clauses, .. } => seq(clauses),
        FormKind::Quote(inner)
        | FormKind::SyntaxQuote(inner)
        | FormKind::Unquote(inner)
        | FormKind::UnquoteSplice(inner)
        | FormKind::Deref(inner)
        | FormKind::Var(inner)
        | FormKind::TaggedLiteral(_, inner) => qualified_under(inner, prefix),
        FormKind::Meta(meta, inner) => {
            qualified_under(meta, prefix) + qualified_under(inner, prefix)
        }
        _ => 0,
    }
}

fn auto_count(f: &Form) -> usize {
    unresolved(f)
}
