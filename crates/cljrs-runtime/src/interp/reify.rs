//! `reify`: an anonymous datatype whose methods close over call-site locals.
//!
//! The `reify` macro emits `(deftype* T :reify Proto (method [this ...] body) ...)`.
//! One tag `T` per reify FORM; the locals the bodies reference become fields
//! of each INSTANCE, so two instances built by one form at different times
//! each answer with their own captured values.

use std::collections::HashSet;
use std::sync::Arc;

use crate::env::env::Env;
use crate::env::error::EvalResult;
use crate::interp::special::{DEFTYPE_SELF, register_impls_for_tag};
use cljrs_gc::GcPtr;
use cljrs_reader::Form;
use cljrs_reader::form::FormKind;
use cljrs_value::value::DatatypeKind;
use cljrs_value::{Keyword, MapValue, TypeInstance, Value};

/// Register `impls` under `type_tag` with every call-site local they
/// reference bound as an instance field, and return a fresh reify instance
/// holding this evaluation's values of those locals.
pub(crate) fn eval_reify_star(type_tag: &Arc<str>, impls: &[Form], env: &mut Env) -> EvalResult {
    let captured = captured_locals(impls, env);
    let field_names: Vec<Arc<str>> = captured.iter().map(|(n, _)| n.clone()).collect();
    register_impls_for_tag(type_tag, impls, &field_names, &[], env)?;
    let fields = MapValue::from_pairs(
        captured
            .into_iter()
            .map(|(n, v)| (Value::keyword(Keyword::parse(&n)), v))
            .collect::<Vec<_>>(),
    );
    Ok(Value::TypeInstance(GcPtr::new(TypeInstance {
        type_tag: type_tag.clone(),
        fields,
        mutable: None,
        kind: DatatypeKind::Reify,
    })))
}

/// The locals in scope that `impls` name, innermost binding winning, in first
/// binding order.
fn captured_locals(impls: &[Form], env: &Env) -> Vec<(Arc<str>, Value)> {
    let mut referenced: HashSet<String> = HashSet::new();
    for form in impls {
        collect_symbols(form, &mut referenced);
    }
    // `set!` of an enclosing deftype's mutable field reaches its instance
    // through this hidden local, which no body names.
    referenced.insert(DEFTYPE_SELF.to_string());

    let (names, vals) = env.all_local_bindings();
    let mut captured: Vec<(Arc<str>, Value)> = Vec::new();
    for (name, val) in names.into_iter().zip(vals) {
        if !referenced.contains(name.as_ref()) {
            continue;
        }
        // Frames come outermost first, so a later binding is the inner one.
        match captured.iter_mut().find(|(n, _)| *n == name) {
            Some(slot) => slot.1 = val,
            None => captured.push((name, val)),
        }
    }
    captured
}

/// The hidden name a method impl's `this` takes when a later param reuses the
/// name the source gave it.
const SHADOWED_THIS: &str = "__impl_this__";

/// For a datatype method params vector whose first (`this`) symbol is reused
/// by a later param, as in `[_ _]`, the same vector with the first renamed to
/// a hidden symbol, so fields still read from the instance. `None` otherwise.
/// The source name stays bound to the later param, as on the JVM.
pub(crate) fn unshadow_this(params: &Form) -> Option<Form> {
    let FormKind::Vector(ps) = &params.kind else {
        return None;
    };
    let FormKind::Symbol(this) = &ps.first()?.kind else {
        return None;
    };
    let reused = ps[1..]
        .iter()
        .any(|p| matches!(&p.kind, FormKind::Symbol(s) if s == this));
    if !reused {
        return None;
    }
    let mut renamed = ps.clone();
    renamed[0] = Form::new(
        FormKind::Symbol(SHADOWED_THIS.to_string()),
        ps[0].span.clone(),
    );
    Some(Form::new(FormKind::Vector(renamed), params.span.clone()))
}

/// Every symbol written in `form`, outside quoted data.
fn collect_symbols(form: &Form, out: &mut HashSet<String>) {
    match &form.kind {
        FormKind::Symbol(s) => {
            out.insert(s.clone());
        }
        FormKind::List(v)
        | FormKind::Vector(v)
        | FormKind::Map(v)
        | FormKind::Set(v)
        | FormKind::AnonFn(v) => v.iter().for_each(|f| collect_symbols(f, out)),
        FormKind::ReaderCond { clauses, .. } => {
            clauses.iter().for_each(|f| collect_symbols(f, out))
        }
        FormKind::SyntaxQuote(f)
        | FormKind::Unquote(f)
        | FormKind::UnquoteSplice(f)
        | FormKind::Deref(f)
        | FormKind::Var(f)
        | FormKind::TaggedLiteral(_, f) => collect_symbols(f, out),
        FormKind::Meta(m, f) => {
            collect_symbols(m, out);
            collect_symbols(f, out);
        }
        _ => {}
    }
}
