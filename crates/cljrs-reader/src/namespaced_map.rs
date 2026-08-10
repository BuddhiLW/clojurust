//! Key qualification for namespaced map literals (`#:ns{…}`, `#::{…}`,
//! `#::alias{…}`).
//!
//! Pure: the whole feature is a source-level rewrite of the keys of an
//! already-parsed map, so it is expressed as a function over `Form` with no
//! reader state. Auto-resolved spellings are NOT resolved here — they lower to
//! `AutoKeyword`, which the evaluator already resolves against the current
//! namespace and its aliases. That keeps namespace resolution in exactly one
//! place instead of teaching the reader about `*ns*`.

use crate::form::{Form, FormKind};

/// Qualify the keys of a namespaced map literal's body.
///
/// `forms` is the flat key/value vector of the map; only even indices are
/// touched. `ns` is the literal namespace (`#:ns`), the alias (`#::alias`), or
/// `""` for the bare auto-resolved form (`#::`).
pub fn qualify_keys(ns: &str, auto: bool, forms: Vec<Form>) -> Result<Vec<Form>, String> {
    forms
        .into_iter()
        .enumerate()
        .map(|(i, form)| {
            if i % 2 == 0 {
                qualify_key(ns, auto, form)
            } else {
                Ok(form)
            }
        })
        .collect()
}

/// Qualify one key. Keys that already carry a namespace, and key forms that are
/// neither a keyword nor a symbol, pass through untouched — matching the JVM
/// reader.
fn qualify_key(ns: &str, auto: bool, key: Form) -> Result<Form, String> {
    let span = key.span.clone();
    let kind = match key.kind {
        FormKind::Keyword(name) => match qualified(ns, auto, &name) {
            Qualified::Unchanged => FormKind::Keyword(name),
            Qualified::Literal(full) => FormKind::Keyword(full),
            Qualified::Auto(full) => FormKind::AutoKeyword(full),
        },
        FormKind::Symbol(name) => match qualified(ns, auto, &name) {
            Qualified::Unchanged => FormKind::Symbol(name),
            Qualified::Literal(full) => FormKind::Symbol(full),
            // There is no auto-resolved SYMBOL form to lower to: `::sym` is not
            // Clojure syntax, so nothing downstream knows how to resolve one.
            // Refusing is the honest answer — silently leaving the key bare
            // would read as success and produce the wrong map.
            Qualified::Auto(_) => {
                return Err(format!(
                    "auto-resolved namespaced map cannot qualify the symbol key '{name}' \
                     — write the namespace explicitly (#:my.ns{{{name} …}})"
                ));
            }
        },
        other => other,
    };
    Ok(Form::new(kind, span))
}

/// What qualifying a bare key name yields.
enum Qualified {
    /// Already namespaced (or not qualifiable) — leave the key alone.
    Unchanged,
    /// A fully-spelled `ns/name`.
    Literal(String),
    /// An auto-resolved `alias/name` (or bare `name` for `#::`).
    Auto(String),
}

fn qualified(ns: &str, auto: bool, name: &str) -> Qualified {
    // `/` is the division symbol, not a namespace separator.
    if name == "/" {
        return Qualified::Unchanged;
    }
    match name.split_once('/') {
        // `:_/k` explicitly opts the key OUT of the map's namespace.
        Some(("_", bare)) => Qualified::Literal(bare.to_string()),
        // Any other explicit namespace wins over the map's.
        Some(_) => Qualified::Unchanged,
        None if auto && ns.is_empty() => Qualified::Auto(name.to_string()),
        None if auto => Qualified::Auto(format!("{ns}/{name}")),
        None => Qualified::Literal(format!("{ns}/{name}")),
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

    fn keys(ns: &str, auto: bool, forms: Vec<Form>) -> Vec<FormKind> {
        qualify_keys(ns, auto, forms)
            .unwrap()
            .into_iter()
            .map(|x| x.kind)
            .collect()
    }

    #[test]
    fn literal_ns_qualifies_bare_keyword_keys() {
        let got = keys("adt", false, vec![kw("a"), f(FormKind::Int(1))]);
        assert_eq!(got[0], FormKind::Keyword("adt/a".to_string()));
        assert_eq!(got[1], FormKind::Int(1));
    }

    #[test]
    fn values_are_never_touched() {
        // A value that LOOKS like a bare key must survive unqualified.
        let got = keys("adt", false, vec![kw("a"), kw("b")]);
        assert_eq!(got[1], FormKind::Keyword("b".to_string()));
    }

    #[test]
    fn explicit_namespace_on_a_key_wins() {
        let got = keys("adt", false, vec![kw("other/a"), f(FormKind::Int(1))]);
        assert_eq!(got[0], FormKind::Keyword("other/a".to_string()));
    }

    #[test]
    fn underscore_namespace_unqualifies() {
        let got = keys("adt", false, vec![kw("_/a"), f(FormKind::Int(1))]);
        assert_eq!(got[0], FormKind::Keyword("a".to_string()));
    }

    #[test]
    fn bare_auto_lowers_to_auto_keyword() {
        let got = keys("", true, vec![kw("a"), f(FormKind::Int(1))]);
        assert_eq!(got[0], FormKind::AutoKeyword("a".to_string()));
    }

    #[test]
    fn aliased_auto_lowers_to_auto_keyword_with_the_alias() {
        let got = keys("al", true, vec![kw("a"), f(FormKind::Int(1))]);
        assert_eq!(got[0], FormKind::AutoKeyword("al/a".to_string()));
    }

    #[test]
    fn symbol_keys_qualify_under_a_literal_namespace() {
        let got = keys(
            "adt",
            false,
            vec![f(FormKind::Symbol("a".to_string())), f(FormKind::Int(1))],
        );
        assert_eq!(got[0], FormKind::Symbol("adt/a".to_string()));
    }

    #[test]
    fn division_symbol_key_is_left_alone() {
        let got = keys(
            "adt",
            false,
            vec![f(FormKind::Symbol("/".to_string())), f(FormKind::Int(1))],
        );
        assert_eq!(got[0], FormKind::Symbol("/".to_string()));
    }

    #[test]
    fn non_identifier_keys_pass_through() {
        let got = keys("adt", false, vec![f(FormKind::Int(7)), f(FormKind::Int(1))]);
        assert_eq!(got[0], FormKind::Int(7));
    }

    #[test]
    fn auto_resolved_symbol_key_is_refused_rather_than_silently_wrong() {
        let err = qualify_keys(
            "",
            true,
            vec![f(FormKind::Symbol("a".to_string())), f(FormKind::Int(1))],
        )
        .unwrap_err();
        assert!(err.contains("auto-resolved"), "{err}");
    }
}
