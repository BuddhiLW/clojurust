// ── form_to_value ─────────────────────────────────────────────────────────────

use cljrs_gc::GcPtr;
use cljrs_reader::{Form, FormKind};
use cljrs_value::value::SetValue;
use cljrs_value::{
    Keyword, MapValue, PersistentHashSet, PersistentList, PersistentVector, Symbol, Value,
};
use regex::Regex;

// ── anon fn expansion ─────────────────────────────────────────────────────────

/// Expand `#(...)` to `(fn* [p__1 p__2 ... & rest__] ...)`.
pub fn expand_anon_fn(body: &[Form], span: cljrs_types::span::Span) -> Form {
    let mut max_pos: usize = 0;
    let mut has_rest = false;
    find_pct_refs(body, &mut max_pos, &mut has_rest);

    let s = &span;
    let mut params: Vec<Form> = (1..=max_pos)
        .map(|i| Form::new(FormKind::Symbol(format!("p__{i}")), s.clone()))
        .collect();
    if has_rest {
        params.push(Form::new(FormKind::Symbol("&".into()), s.clone()));
        params.push(Form::new(FormKind::Symbol("rest__".into()), s.clone()));
    }

    let new_body = rewrite_pct_refs(body, s.clone());

    // Wrap the rewritten body forms back into a single call expression.
    // #(f a b) → (fn* [params] (f a b)), not (fn* [params] f a b).
    let body_expr = Form::new(FormKind::List(new_body), s.clone());

    Form::new(
        FormKind::List(vec![
            Form::new(FormKind::Symbol("fn*".into()), s.clone()),
            Form::new(FormKind::Vector(params), s.clone()),
            body_expr,
        ]),
        span,
    )
}

fn find_pct_refs(forms: &[Form], max_pos: &mut usize, has_rest: &mut bool) {
    for form in forms {
        find_pct_refs_form(form, max_pos, has_rest);
    }
}

fn find_pct_refs_form(form: &Form, max_pos: &mut usize, has_rest: &mut bool) {
    match &form.kind {
        FormKind::Symbol(s) if (s == "%" || s == "%1") && *max_pos < 1 => {
            *max_pos = 1;
        }
        FormKind::Symbol(s) if s == "%&" => {
            *has_rest = true;
        }
        FormKind::Symbol(s) if s.starts_with('%') => {
            if let Ok(n) = s[1..].parse::<usize>()
                && n > *max_pos
            {
                *max_pos = n;
            }
        }
        FormKind::List(c) | FormKind::Vector(c) | FormKind::Set(c) | FormKind::Map(c) => {
            find_pct_refs(c, max_pos, has_rest);
        }
        // Reader-macro sugar (`@%`, `#'%`, `'%`, `` `% ``, `~%`, `~@%`, `#tag %`)
        // wraps a single inner form; it must be scanned the same as any
        // other nested form so `%` refs under sugar aren't missed.
        FormKind::Quote(inner)
        | FormKind::SyntaxQuote(inner)
        | FormKind::Unquote(inner)
        | FormKind::UnquoteSplice(inner)
        | FormKind::Deref(inner)
        | FormKind::Var(inner)
        | FormKind::TaggedLiteral(_, inner) => {
            find_pct_refs_form(inner, max_pos, has_rest);
        }
        FormKind::Meta(meta, inner) => {
            find_pct_refs_form(meta, max_pos, has_rest);
            find_pct_refs_form(inner, max_pos, has_rest);
        }
        FormKind::ReaderCond { clauses, .. } => {
            find_pct_refs(clauses, max_pos, has_rest);
        }
        _ => {}
    }
}

fn rewrite_pct_refs(forms: &[Form], span: cljrs_types::span::Span) -> Vec<Form> {
    forms
        .iter()
        .map(|f| rewrite_pct_form(f, span.clone()))
        .collect()
}

fn rewrite_pct_form(form: &Form, span: cljrs_types::span::Span) -> Form {
    match &form.kind {
        FormKind::Symbol(s) if s == "%" || s == "%1" => {
            Form::new(FormKind::Symbol("p__1".into()), span)
        }
        FormKind::Symbol(s) if s == "%&" => Form::new(FormKind::Symbol("rest__".into()), span),
        FormKind::Symbol(s) if s.starts_with('%') => {
            if let Ok(n) = s[1..].parse::<usize>() {
                Form::new(FormKind::Symbol(format!("p__{n}")), span)
            } else {
                form.clone()
            }
        }
        FormKind::List(c) => {
            let rewritten = rewrite_pct_refs(c, span.clone());
            Form::new(FormKind::List(rewritten), span)
        }
        FormKind::Vector(c) => {
            let rewritten = rewrite_pct_refs(c, span.clone());
            Form::new(FormKind::Vector(rewritten), span)
        }
        FormKind::Set(c) => {
            let rewritten = rewrite_pct_refs(c, span.clone());
            Form::new(FormKind::Set(rewritten), span)
        }
        FormKind::Map(c) => {
            let rewritten = rewrite_pct_refs(c, span.clone());
            Form::new(FormKind::Map(rewritten), span)
        }
        FormKind::Quote(inner) => Form::new(
            FormKind::Quote(Box::new(rewrite_pct_form(inner, span.clone()))),
            span,
        ),
        FormKind::SyntaxQuote(inner) => Form::new(
            FormKind::SyntaxQuote(Box::new(rewrite_pct_form(inner, span.clone()))),
            span,
        ),
        FormKind::Unquote(inner) => Form::new(
            FormKind::Unquote(Box::new(rewrite_pct_form(inner, span.clone()))),
            span,
        ),
        FormKind::UnquoteSplice(inner) => Form::new(
            FormKind::UnquoteSplice(Box::new(rewrite_pct_form(inner, span.clone()))),
            span,
        ),
        FormKind::Deref(inner) => Form::new(
            FormKind::Deref(Box::new(rewrite_pct_form(inner, span.clone()))),
            span,
        ),
        FormKind::Var(inner) => Form::new(
            FormKind::Var(Box::new(rewrite_pct_form(inner, span.clone()))),
            span,
        ),
        FormKind::TaggedLiteral(tag, inner) => Form::new(
            FormKind::TaggedLiteral(tag.clone(), Box::new(rewrite_pct_form(inner, span.clone()))),
            span,
        ),
        FormKind::Meta(meta, inner) => Form::new(
            FormKind::Meta(
                Box::new(rewrite_pct_form(meta, span.clone())),
                Box::new(rewrite_pct_form(inner, span.clone())),
            ),
            span,
        ),
        FormKind::ReaderCond { splicing, clauses } => Form::new(
            FormKind::ReaderCond {
                splicing: *splicing,
                clauses: rewrite_pct_refs(clauses, span.clone()),
            },
            span,
        ),
        _ => form.clone(),
    }
}

/// Convert a `Form` to its literal `Value` without evaluating.
/// Used by `quote` and macro expansion.
pub fn form_to_value(form: &Form) -> Value {
    match &form.kind {
        FormKind::Nil => Value::Nil,
        FormKind::Bool(b) => Value::Bool(*b),
        FormKind::Int(n) => Value::Long(*n),
        FormKind::Float(f) => Value::Double(*f),
        FormKind::Symbolic(f) => Value::Double(*f),
        FormKind::Str(s) => Value::string(s.clone()),
        FormKind::Char(c) => Value::Char(*c),
        FormKind::BigInt(s) => crate::parse_bigint(s).unwrap_or(Value::Nil),
        FormKind::BigDecimal(s) => crate::parse_bigdecimal(s).unwrap_or(Value::Nil),
        FormKind::Ratio(s) => crate::parse_ratio(s).unwrap_or(Value::Nil),

        FormKind::Symbol(s) => Value::symbol(Symbol::parse(s)),
        FormKind::Keyword(s) => Value::keyword(Keyword::parse(s)),
        FormKind::AutoKeyword(s) => Value::keyword(Keyword::simple(s.as_str())),
        FormKind::Regex(s) => match Regex::new(s.as_str()) {
            Ok(pattern) => Value::Pattern(GcPtr::new(pattern)),
            Err(_) => Value::Nil, // should already have been caught
        },

        FormKind::List(forms) => {
            let expanded = expand_reader_conds(forms);
            let items: Vec<Value> = expanded.iter().map(form_to_value).collect();
            Value::List(GcPtr::new(PersistentList::from_iter(items)))
        }
        FormKind::Vector(forms) => {
            let expanded = expand_reader_conds(forms);
            let items: Vec<Value> = expanded.iter().map(form_to_value).collect();
            Value::Vector(GcPtr::new(PersistentVector::from_iter(items)))
        }
        FormKind::Map(forms) => {
            let mut m = MapValue::empty();
            for pair in forms.chunks(2) {
                if pair.len() == 2 {
                    m = m.assoc(form_to_value(&pair[0]), form_to_value(&pair[1]));
                }
            }
            Value::Map(m)
        }
        FormKind::Set(forms) => {
            let s = forms
                .iter()
                .fold(PersistentHashSet::empty(), |s, f| s.conj(form_to_value(f)));
            Value::Set(SetValue::Hash(GcPtr::new(s)))
        }

        FormKind::Quote(inner) => {
            // `'x` → the form x as a data value.
            Value::List(GcPtr::new(PersistentList::from_iter([
                Value::symbol(Symbol::simple("quote")),
                form_to_value(inner),
            ])))
        }
        FormKind::SyntaxQuote(inner) => Value::List(GcPtr::new(PersistentList::from_iter([
            Value::symbol(Symbol::simple("syntax-quote")),
            form_to_value(inner),
        ]))),
        FormKind::Unquote(inner) => Value::List(GcPtr::new(PersistentList::from_iter([
            Value::symbol(Symbol::simple("unquote")),
            form_to_value(inner),
        ]))),
        FormKind::UnquoteSplice(inner) => Value::List(GcPtr::new(PersistentList::from_iter([
            Value::symbol(Symbol::simple("unquote-splicing")),
            form_to_value(inner),
        ]))),
        FormKind::Deref(inner) => Value::List(GcPtr::new(PersistentList::from_iter([
            Value::symbol(Symbol::simple("deref")),
            form_to_value(inner),
        ]))),
        FormKind::Var(inner) => Value::List(GcPtr::new(PersistentList::from_iter([
            Value::symbol(Symbol::simple("var")),
            form_to_value(inner),
        ]))),
        FormKind::Meta(_meta, inner) => form_to_value(inner),
        FormKind::AnonFn(body) => {
            // Expand #(...) to (fn* [...] ...) so it round-trips correctly through quote.
            let expanded = expand_anon_fn(body, form.span.clone());
            form_to_value(&expanded)
        }
        FormKind::TaggedLiteral(tag, inner) => match tag.as_str() {
            "uuid" => {
                if let FormKind::Str(s) = &inner.kind {
                    match uuid::Uuid::parse_str(s) {
                        Ok(u) => Value::Uuid(u.as_u128()),
                        Err(_) => form_to_value(inner),
                    }
                } else {
                    form_to_value(inner)
                }
            }
            _ => form_to_value(inner),
        },
        FormKind::ReaderCond {
            splicing: false,
            clauses,
        } => select_reader_cond(clauses).map_or(Value::Nil, form_to_value),
        FormKind::ReaderCond { splicing: true, .. } => Value::Nil, // splice must be handled by parent
    }
}

/// Resolve a `#?(...)` reader conditional to the selected branch form, or
/// `None` if no `:rust` or `:default` clause is present.
pub fn select_reader_cond(clauses: &[Form]) -> Option<&Form> {
    let mut default: Option<&Form> = None;
    for pair in clauses.chunks_exact(2) {
        let (feature, branch) = (&pair[0], &pair[1]);
        match &feature.kind {
            FormKind::Keyword(k) if k == "rust" => return Some(branch),
            FormKind::Keyword(k) if k == "default" => default = Some(branch),
            _ => {}
        }
    }
    default
}

/// Expand reader conditionals in a flat slice of forms.
///
/// - Non-splicing `#?(...)`: replaced by the selected branch (or removed if none).
/// - Splicing `#?@(...)`: selected branch must be a vector/list; its elements
///   are inlined.  If no branch matches, the splice is removed (empty).
pub fn expand_reader_conds(forms: &[Form]) -> Vec<Form> {
    let mut out = Vec::with_capacity(forms.len());
    for form in forms {
        match &form.kind {
            FormKind::ReaderCond {
                splicing: true,
                clauses,
            } => {
                if let Some(selected) = select_reader_cond(clauses) {
                    match &selected.kind {
                        FormKind::Vector(elems) | FormKind::List(elems) => {
                            // Recursively expand any nested reader conditionals
                            // within the spliced elements.
                            let expanded_elems = expand_reader_conds(elems);
                            out.extend(expanded_elems);
                        }
                        // Non-sequence branch: inline it as a single element.
                        _ => out.push(selected.clone()),
                    }
                }
                // No matching branch → splice nothing (empty).
            }
            FormKind::ReaderCond {
                splicing: false,
                clauses,
            } => {
                if let Some(selected) = select_reader_cond(clauses) {
                    out.push(selected.clone());
                }
                // No matching branch → omit.
            }
            _ => out.push(form.clone()),
        }
    }
    out
}

/// Expand `#?`/`#?@` reader conditionals in a sibling-form slice, borrowing the
/// input unchanged when none are present.
///
/// Callers that validate element structure (e.g. map key/value parity) must do
/// so on the returned slice.
pub fn expand_reader_conds_cow(forms: &[Form]) -> std::borrow::Cow<'_, [Form]> {
    if forms
        .iter()
        .any(|f| matches!(f.kind, FormKind::ReaderCond { .. }))
    {
        std::borrow::Cow::Owned(expand_reader_conds(forms))
    } else {
        std::borrow::Cow::Borrowed(forms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cljrs_reader::Parser;

    fn parse_anon_fn(src: &str) -> Form {
        let mut parser = Parser::new(src.to_string(), "<test>".to_string());
        let form = parser.parse_one().unwrap().unwrap();
        let FormKind::AnonFn(body) = form.kind else {
            panic!("expected AnonFn, got {:?}", form.kind);
        };
        expand_anon_fn(&body, form.span)
    }

    fn arity(expanded: &Form) -> usize {
        let FormKind::List(parts) = &expanded.kind else {
            panic!("expected (fn* [...] ...)");
        };
        let FormKind::Vector(params) = &parts[1].kind else {
            panic!("expected param vector");
        };
        params.len()
    }

    #[test]
    fn deref_sugar_counts_as_one_arg() {
        // #(:x @%) must expand to (fn* [p__1] (:x (deref p__1))), arity 1 —
        // not 0, as if % under `@` were invisible to the arg scanner.
        let expanded = parse_anon_fn("#(:x @%)");
        assert_eq!(arity(&expanded), 1);

        let FormKind::List(parts) = &expanded.kind else {
            unreachable!()
        };
        let FormKind::List(body) = &parts[2].kind else {
            panic!("expected body list");
        };
        let FormKind::Deref(inner) = &body[1].kind else {
            panic!("expected deref form, got {:?}", body[1].kind);
        };
        assert_eq!(inner.kind, FormKind::Symbol("p__1".to_string()));
    }

    #[test]
    fn var_and_meta_sugar_also_scanned() {
        assert_eq!(arity(&parse_anon_fn("#(#'%)")), 1);
        assert_eq!(arity(&parse_anon_fn("#(^:x %)")), 1);
        assert_eq!(arity(&parse_anon_fn("#('%)")), 1);
    }

    // ── reader-conditional expansion (#?@ splice) ───────────────────────────

    fn parse_one_form(src: &str) -> Form {
        let mut p = Parser::new(src.to_string(), "<test>".to_string());
        p.parse_one().unwrap().unwrap()
    }

    fn vec_elems(src: &str) -> Vec<Form> {
        match parse_one_form(src).kind {
            FormKind::Vector(v) => v,
            other => panic!("expected vector literal, got {other:?}"),
        }
    }

    fn kinds(forms: &[Form]) -> Vec<FormKind> {
        forms.iter().map(|f| f.kind.clone()).collect()
    }

    /// Expand the elements of a `[...]` literal and return their kinds.
    fn expand_src(src: &str) -> Vec<FormKind> {
        kinds(&expand_reader_conds(&vec_elems(src)))
    }

    #[test]
    fn splice_rust_branch_flattens_into_parent() {
        assert_eq!(
            expand_src("[:x #?@(:rust [:a :b]) :y]"),
            kinds(&vec_elems("[:x :a :b :y]"))
        );
    }

    #[test]
    fn splice_falls_back_to_default_branch() {
        assert_eq!(
            expand_src("[:x #?@(:default [:a :b]) :y]"),
            kinds(&vec_elems("[:x :a :b :y]"))
        );
    }

    #[test]
    fn splice_with_no_matching_branch_is_removed_not_nil() {
        // `:clj` does not match under cljrs; the splice contributes nothing.
        // Regression: eval'ing the conditional on its own used to leave a `nil`.
        assert_eq!(
            expand_src("[:x #?@(:clj [:a :b]) :y]"),
            kinds(&vec_elems("[:x :y]"))
        );
    }

    #[test]
    fn non_splicing_conditional_selects_single_form() {
        assert_eq!(
            expand_src("[:x #?(:rust :a :default :b) :y]"),
            kinds(&vec_elems("[:x :a :y]"))
        );
    }

    #[test]
    fn nested_conditionals_inside_spliced_branch_expand() {
        assert_eq!(
            expand_src("[#?@(:rust [#?(:rust :a :default :z) :b])]"),
            kinds(&vec_elems("[:a :b]"))
        );
    }

    #[test]
    fn cow_borrows_when_no_conditional_present() {
        let forms = vec_elems("[:x :y :z]");
        assert!(matches!(
            expand_reader_conds_cow(&forms),
            std::borrow::Cow::Borrowed(_)
        ));
    }

    #[test]
    fn cow_owns_when_conditional_present() {
        let forms = vec_elems("[:x #?@(:rust [:a]) :y]");
        assert!(matches!(
            expand_reader_conds_cow(&forms),
            std::borrow::Cow::Owned(_)
        ));
    }

    fn kw_list(names: &[String]) -> String {
        names
            .iter()
            .map(|n| format!(":{n}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    // Model-based oracle. An `Item` is one position in a form sequence: a plain
    // keyword, a non-splicing conditional, or a splicing conditional. Each knows
    // both how to render itself as source and what it must contribute after
    // expansion under the `:rust` selection rule - an independent reimplementation
    // of `select_reader_cond` the production code is checked against.

    #[derive(Clone, Debug)]
    enum Item {
        Plain(String),
        NonSplice(Vec<(String, String)>),
        Splice(Vec<(String, Vec<String>)>),
    }

    fn select_model<T>(branches: &[(String, T)]) -> Option<&T> {
        let mut default = None;
        for (k, v) in branches {
            if k == "rust" {
                return Some(v);
            }
            if k == "default" {
                default = Some(v);
            }
        }
        default
    }

    fn item_src(it: &Item) -> String {
        match it {
            Item::Plain(n) => format!(":{n}"),
            Item::NonSplice(brs) => {
                let inner = brs
                    .iter()
                    .map(|(k, n)| format!(":{k} :{n}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                format!("#?({inner})")
            }
            Item::Splice(brs) => {
                let inner = brs
                    .iter()
                    .map(|(k, vs)| format!(":{k} [{}]", kw_list(vs)))
                    .collect::<Vec<_>>()
                    .join(" ");
                format!("#?@({inner})")
            }
        }
    }

    fn item_expected(it: &Item) -> Vec<String> {
        match it {
            Item::Plain(n) => vec![n.clone()],
            Item::NonSplice(brs) => select_model(brs).cloned().into_iter().collect(),
            Item::Splice(brs) => select_model(brs).cloned().unwrap_or_default(),
        }
    }

    fn key_strat() -> impl proptest::strategy::Strategy<Value = String> {
        proptest::prop_oneof![
            proptest::strategy::Just("rust".to_string()),
            proptest::strategy::Just("clj".to_string()),
            proptest::strategy::Just("cljs".to_string()),
            proptest::strategy::Just("default".to_string()),
        ]
    }

    fn item_strat() -> impl proptest::strategy::Strategy<Value = Item> {
        use proptest::collection::vec as pvec;
        use proptest::strategy::Strategy;
        proptest::prop_oneof![
            "[a-z]{1,4}".prop_map(Item::Plain),
            pvec((key_strat(), "[a-z]{1,4}".prop_map(|s| s)), 1..4).prop_map(Item::NonSplice),
            pvec((key_strat(), pvec("[a-z]{1,4}", 0..3)), 1..4).prop_map(Item::Splice),
        ]
    }

    fn seq_src(items: &[Item]) -> String {
        format!(
            "[{}]",
            items.iter().map(item_src).collect::<Vec<_>>().join(" ")
        )
    }

    fn seq_expected_src(items: &[Item]) -> String {
        let names: Vec<String> = items.iter().flat_map(|it| item_expected(it)).collect();
        format!("[{}]", kw_list(&names))
    }

    proptest::proptest! {
        /// Splicing `#?@(:rust xs)` at a position equals inlining `xs` there.
        #[test]
        fn prop_splice_equals_inline(
            pre in proptest::collection::vec("[a-z]{1,4}", 0..4),
            mid in proptest::collection::vec("[a-z]{1,4}", 0..4),
            suf in proptest::collection::vec("[a-z]{1,4}", 0..4),
        ) {
            let spliced = format!("[{} #?@(:rust [{}]) {}]",
                                  kw_list(&pre), kw_list(&mid), kw_list(&suf));
            let inlined = format!("[{} {} {}]",
                                  kw_list(&pre), kw_list(&mid), kw_list(&suf));
            proptest::prop_assert_eq!(expand_src(&spliced), kinds(&vec_elems(&inlined)));
        }

        /// Expansion is idempotent: a second pass changes nothing.
        #[test]
        fn prop_expand_idempotent(
            pre in proptest::collection::vec("[a-z]{1,4}", 0..4),
            mid in proptest::collection::vec("[a-z]{1,4}", 0..4),
        ) {
            let src = format!("[{} #?@(:rust [{}])]", kw_list(&pre), kw_list(&mid));
            let once = expand_reader_conds(&vec_elems(&src));
            let twice = expand_reader_conds(&once);
            proptest::prop_assert_eq!(kinds(&once), kinds(&twice));
        }

        /// With no reader conditionals, expansion is the identity.
        #[test]
        fn prop_no_conditional_is_identity(
            xs in proptest::collection::vec("[a-z]{1,4}", 0..8),
        ) {
            let src = format!("[{}]", kw_list(&xs));
            proptest::prop_assert_eq!(expand_src(&src), kinds(&vec_elems(&src)));
        }

        /// Expansion of an arbitrary interleaving of plain / non-splicing /
        /// splicing conditionals (mixed keys) equals the model's expected output.
        #[test]
        fn prop_model_matches_expand(
            items in proptest::collection::vec(item_strat(), 0..6),
        ) {
            proptest::prop_assert_eq!(
                expand_src(&seq_src(&items)),
                kinds(&vec_elems(&seq_expected_src(&items)))
            );
        }

        /// No `ReaderCond` node survives expansion at the top level.
        #[test]
        fn prop_no_reader_cond_survives(
            items in proptest::collection::vec(item_strat(), 0..6),
        ) {
            let expanded = expand_reader_conds(&vec_elems(&seq_src(&items)));
            let none_survive = expanded
                .iter()
                .all(|f| !matches!(f.kind, FormKind::ReaderCond { .. }));
            proptest::prop_assert!(none_survive);
        }

        /// The expanded length equals the sum of each item's contributed count.
        #[test]
        fn prop_length_law(
            items in proptest::collection::vec(item_strat(), 0..6),
        ) {
            let expected_len: usize = items.iter().map(|it| item_expected(it).len()).sum();
            let got = expand_reader_conds(&vec_elems(&seq_src(&items)));
            proptest::prop_assert_eq!(got.len(), expected_len);
        }
    }
}
