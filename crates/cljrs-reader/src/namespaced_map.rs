//! Key qualification for namespaced map literals (`#:ns{…}`, `#::{…}`,
//! `#::alias{…}`).
//!
//! Pure: the whole feature is a source-level rewrite of the keys of an
//! already-parsed map, so it is expressed as functions over `Form` with no
//! reader state. Auto-resolved spellings are NOT resolved here - they lower to
//! `AutoKeyword` / `AutoSymbol`, which the evaluator resolves against the
//! current namespace and its aliases.

use crate::chars::is_symbol_start;
use crate::form::{Form, FormKind};

/// The namespace prefix of a namespaced map literal, after validation.
///
/// The JVM reader reads the prefix as an unqualified `Symbol`, so `#:foo/bar`,
/// `#:1` and `#:nil` are read errors. Parsing into this type performs that
/// check once, and leaves `(namespace, auto)` combinations that cannot occur -
/// such as a bare `#:{…}` - unrepresentable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MapNs {
    /// `#:ns{…}` - the namespace is written out.
    Literal(String),
    /// `#::{…}` - the namespace the form is read in.
    CurrentNs,
    /// `#::alias{…}` - a `:require :as` alias, resolved by the evaluator.
    Alias(String),
}

impl MapNs {
    /// Validate the prefix text following `#:` (or `#::` when `auto`).
    ///
    /// `Err` carries a reader-facing message.
    pub fn parse(text: &str, auto: bool) -> Result<Self, String> {
        if text.is_empty() {
            return if auto {
                Ok(MapNs::CurrentNs)
            } else {
                Err("namespaced map literal requires a namespace".to_string())
            };
        }
        validate_unqualified_symbol(text)?;
        Ok(if auto {
            MapNs::Alias(text.to_string())
        } else {
            MapNs::Literal(text.to_string())
        })
    }
}

/// A namespaced map's prefix must read as an unqualified symbol.
fn validate_unqualified_symbol(text: &str) -> Result<(), String> {
    let first = text.chars().next().expect("caller checked non-empty");
    if !is_symbol_start(first) {
        return Err(format!(
            "namespaced map prefix must be an unqualified symbol, got '{text}'"
        ));
    }
    if text.contains('/') {
        return Err(format!(
            "namespaced map prefix must be an unqualified symbol, got '{text}' \
             (a prefix carries no namespace of its own)"
        ));
    }
    if matches!(text, "nil" | "true" | "false") {
        return Err(format!(
            "namespaced map prefix must be an unqualified symbol, got '{text}'"
        ));
    }
    Ok(())
}

/// Qualify the keys of a namespaced map literal's body.
///
/// `forms` is the flat key/value vector of the map; only even indices are
/// touched.
pub fn qualify_keys(ns: &MapNs, forms: Vec<Form>) -> Vec<Form> {
    forms
        .into_iter()
        .enumerate()
        .map(|(i, form)| {
            if i.is_multiple_of(2) {
                qualify_key(ns, form)
            } else {
                form
            }
        })
        .collect()
}

/// Qualify one key. Keys that already carry a namespace, and key forms that are
/// neither a keyword nor a symbol, pass through untouched - matching the JVM
/// reader.
fn qualify_key(ns: &MapNs, key: Form) -> Form {
    let span = key.span.clone();
    let kind = match key.kind {
        FormKind::Keyword(name) => match qualified(ns, &name) {
            Qualified::Unchanged => FormKind::Keyword(name),
            Qualified::Literal(full) => FormKind::Keyword(full),
            Qualified::Auto(full) => FormKind::AutoKeyword(full),
        },
        FormKind::Symbol(name) => match qualified(ns, &name) {
            Qualified::Unchanged => FormKind::Symbol(name),
            Qualified::Literal(full) => FormKind::Symbol(full),
            Qualified::Auto(full) => FormKind::AutoSymbol(full),
        },
        other => other,
    };
    Form::new(kind, span)
}

/// What qualifying a bare key name yields.
enum Qualified {
    /// Already namespaced - leave the key alone.
    Unchanged,
    /// A fully-spelled `ns/name`.
    Literal(String),
    /// An auto-resolved `alias/name` (or bare `name` for `#::`).
    Auto(String),
}

fn qualified(ns: &MapNs, name: &str) -> Qualified {
    // `/` is a name, not a separator: `#:foo{/ 1}` has the key `foo//`.
    let already_namespaced = match name.split_once('/') {
        // `:_/k` explicitly opts the key OUT of the map's namespace.
        Some(("_", bare)) => return Qualified::Literal(bare.to_string()),
        Some(_) if name != "/" => true,
        _ => false,
    };
    if already_namespaced {
        return Qualified::Unchanged;
    }
    match ns {
        MapNs::CurrentNs => Qualified::Auto(name.to_string()),
        MapNs::Alias(alias) => Qualified::Auto(format!("{alias}/{name}")),
        MapNs::Literal(literal) => Qualified::Literal(format!("{literal}/{name}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cljrs_types::span::Span;
    use std::sync::Arc;

    fn f(kind: FormKind) -> Form {
        Form::new(kind, Span::new(Arc::new("<test>".to_string()), 0, 0, 1, 1))
    }

    fn kw(s: &str) -> Form {
        f(FormKind::Keyword(s.to_string()))
    }

    fn sym(s: &str) -> Form {
        f(FormKind::Symbol(s.to_string()))
    }

    fn keys(ns: MapNs, forms: Vec<Form>) -> Vec<FormKind> {
        qualify_keys(&ns, forms)
            .into_iter()
            .map(|x| x.kind)
            .collect()
    }

    fn lit(ns: &str) -> MapNs {
        MapNs::Literal(ns.to_string())
    }

    // ── prefix validation ───────────────────────────────────────────────────

    #[test]
    fn a_bare_prefix_is_only_legal_when_auto_resolved() {
        assert_eq!(MapNs::parse("", true).unwrap(), MapNs::CurrentNs);
        assert!(
            MapNs::parse("", false)
                .unwrap_err()
                .contains("requires a namespace")
        );
    }

    #[test]
    fn prefix_must_be_an_unqualified_symbol() {
        for (text, auto) in [
            ("foo/bar", false),
            ("foo/bar", true),
            ("1", false),
            ("nil", false),
            ("true", false),
            ("false", false),
            (":kw", false),
        ] {
            let err = match MapNs::parse(text, auto) {
                Ok(ns) => panic!("{text} was accepted as a prefix: {ns:?}"),
                Err(e) => e,
            };
            assert!(err.contains("unqualified symbol"), "{text}: {err}");
        }
    }

    #[test]
    fn ordinary_prefixes_parse() {
        assert_eq!(MapNs::parse("adt", false).unwrap(), lit("adt"));
        assert_eq!(
            MapNs::parse("my.ns", false).unwrap(),
            MapNs::Literal("my.ns".to_string())
        );
        assert_eq!(
            MapNs::parse("al", true).unwrap(),
            MapNs::Alias("al".to_string())
        );
    }

    // ── key qualification ───────────────────────────────────────────────────

    #[test]
    fn literal_ns_qualifies_bare_keyword_keys() {
        let got = keys(lit("adt"), vec![kw("a"), f(FormKind::Int(1))]);
        assert_eq!(got[0], FormKind::Keyword("adt/a".to_string()));
        assert_eq!(got[1], FormKind::Int(1));
    }

    #[test]
    fn values_are_never_touched() {
        // A value that LOOKS like a bare key must survive unqualified.
        let got = keys(lit("adt"), vec![kw("a"), kw("b")]);
        assert_eq!(got[1], FormKind::Keyword("b".to_string()));
    }

    #[test]
    fn explicit_namespace_on_a_key_wins() {
        let got = keys(lit("adt"), vec![kw("other/a"), f(FormKind::Int(1))]);
        assert_eq!(got[0], FormKind::Keyword("other/a".to_string()));
    }

    #[test]
    fn underscore_namespace_unqualifies() {
        let got = keys(lit("adt"), vec![kw("_/a"), f(FormKind::Int(1))]);
        assert_eq!(got[0], FormKind::Keyword("a".to_string()));
    }

    #[test]
    fn bare_auto_lowers_to_auto_keyword() {
        let got = keys(MapNs::CurrentNs, vec![kw("a"), f(FormKind::Int(1))]);
        assert_eq!(got[0], FormKind::AutoKeyword("a".to_string()));
    }

    #[test]
    fn aliased_auto_lowers_to_auto_keyword_with_the_alias() {
        let got = keys(
            MapNs::Alias("al".to_string()),
            vec![kw("a"), f(FormKind::Int(1))],
        );
        assert_eq!(got[0], FormKind::AutoKeyword("al/a".to_string()));
    }

    #[test]
    fn symbol_keys_qualify_under_a_literal_namespace() {
        let got = keys(lit("adt"), vec![sym("a"), f(FormKind::Int(1))]);
        assert_eq!(got[0], FormKind::Symbol("adt/a".to_string()));
    }

    #[test]
    fn symbol_keys_lower_to_auto_symbol_under_an_auto_resolved_namespace() {
        let got = keys(MapNs::CurrentNs, vec![sym("a"), f(FormKind::Int(1))]);
        assert_eq!(got[0], FormKind::AutoSymbol("a".to_string()));

        let got = keys(
            MapNs::Alias("al".to_string()),
            vec![sym("a"), f(FormKind::Int(1))],
        );
        assert_eq!(got[0], FormKind::AutoSymbol("al/a".to_string()));
    }

    #[test]
    fn slash_is_a_name_and_takes_the_map_namespace() {
        // `#:foo{/ 1}` has the symbol key `foo//`, `#:foo{:/ 1}` the keyword
        // key `:foo//` - `/` is a name, not a namespace separator.
        let got = keys(lit("foo"), vec![sym("/"), f(FormKind::Int(1))]);
        assert_eq!(got[0], FormKind::Symbol("foo//".to_string()));

        let got = keys(lit("foo"), vec![kw("/"), f(FormKind::Int(1))]);
        assert_eq!(got[0], FormKind::Keyword("foo//".to_string()));
    }

    #[test]
    fn non_identifier_keys_pass_through() {
        let got = keys(lit("adt"), vec![f(FormKind::Int(7)), f(FormKind::Int(1))]);
        assert_eq!(got[0], FormKind::Int(7));
    }
}
