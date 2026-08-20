//! Known-function name → KnownFn dispatch table.
//!
//! Maps `clojure.core` symbol names to `KnownFn` variants, plus the two pieces
//! that decide whether a symbol names core at all: [`core_name`] (qualifier
//! check) and [`CoreShadows`] (what the lowering namespace binds).

use std::collections::HashSet;
use std::sync::Arc;

use crate::KnownFn;

/// The bare `clojure.core` name `sym` denotes, or `None` when it names
/// something else.
///
/// An unqualified symbol is a candidate (the caller still has to rule out a
/// local or a namespace-level shadow — see [`CoreShadows`]), and an explicit
/// `clojure.core/x` always is one.  Any other qualifier names a different
/// namespace's var: `my.ns/count` is not core's `count`, so it must not be
/// lowered as one.
pub fn core_name(sym: &str) -> Option<&str> {
    // `/` is division, not a namespace separator; `clojure.core//` is that
    // same var written qualified.
    if sym == "/" {
        return Some(sym);
    }
    if let Some(ns) = sym.strip_suffix("//") {
        return (ns == "clojure.core").then(|| &sym[sym.len() - 1..]);
    }
    match sym.rfind('/') {
        None => Some(sym),
        Some(pos) => {
            let (ns, name) = (&sym[..pos], &sym[pos + 1..]);
            (ns == "clojure.core" && !name.is_empty()).then_some(name)
        }
    }
}

/// Names the namespace being lowered does *not* resolve to `clojure.core`.
///
/// A `def` in the namespace itself, a `:refer` of the same name from another
/// namespace, and a `(:refer-clojure :exclude [...])` clause all mean an
/// unqualified call to that name is not core's — so the lowerer must leave it
/// as a dynamic call instead of inlining core's semantics (which is what the
/// tree-walking interpreter does, and the two tiers have to agree).
///
/// The empty set means "no namespace information available": every core name
/// is assumed to resolve to core, which is what lowering did before shadows
/// were threaded through.  Callers that hold an environment
/// (`cljrs_runtime::tiered::lower::core_shadows_for`) should always build a
/// real one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CoreShadows {
    names: Arc<HashSet<Arc<str>>>,
}

impl CoreShadows {
    /// No shadowing information — every core name resolves to core.
    pub fn none() -> Self {
        Self::default()
    }

    pub fn new(names: impl IntoIterator<Item = Arc<str>>) -> Self {
        Self {
            names: Arc::new(names.into_iter().collect()),
        }
    }

    /// Does the lowering namespace resolve `name` to something other than
    /// `clojure.core/name`?
    pub fn contains(&self, name: &str) -> bool {
        self.names.contains(name)
    }

    /// This set plus `extra` — used to fold in shadows only the forms being
    /// lowered know about (see `anf::collect_defined_names`).
    pub fn union(&self, extra: impl IntoIterator<Item = Arc<str>>) -> Self {
        let mut names: HashSet<Arc<str>> = extra.into_iter().collect();
        if names.is_empty() {
            return self.clone();
        }
        names.extend(self.names.iter().cloned());
        Self {
            names: Arc::new(names),
        }
    }
}

/// Resolve a `clojure.core` name to a `KnownFn`, or `None` if not recognized.
///
/// The name must already be unqualified — [`core_name`] establishes that a
/// symbol names core at all, and callers in call position must additionally
/// rule out locals and namespace-level shadowing (`LowerCtx::core_call_name`).
pub fn resolve_known_core_fn(bare_name: &str) -> Option<KnownFn> {
    match bare_name {
        "vector" => Some(KnownFn::Vector),
        "hash-map" => Some(KnownFn::HashMap),
        "hash-set" => Some(KnownFn::HashSet),
        "list" => Some(KnownFn::List),
        "assoc" => Some(KnownFn::Assoc),
        "dissoc" => Some(KnownFn::Dissoc),
        "conj" => Some(KnownFn::Conj),
        "disj" => Some(KnownFn::Disj),
        "get" => Some(KnownFn::Get),
        "nth" => Some(KnownFn::Nth),
        "count" => Some(KnownFn::Count),
        "contains?" => Some(KnownFn::Contains),
        "transient" => Some(KnownFn::Transient),
        "assoc!" => Some(KnownFn::AssocBang),
        "conj!" => Some(KnownFn::ConjBang),
        "persistent!" => Some(KnownFn::PersistentBang),
        "first" => Some(KnownFn::First),
        "rest" => Some(KnownFn::Rest),
        "next" => Some(KnownFn::Next),
        "cons" => Some(KnownFn::Cons),
        "seq" => Some(KnownFn::Seq),
        "lazy-seq" => Some(KnownFn::LazySeq),
        "make-lazy-seq" => Some(KnownFn::LazySeq),
        "empty?" => Some(KnownFn::IsEmpty),
        "peek" => Some(KnownFn::Peek),
        "pop" => Some(KnownFn::Pop),
        "vec" => Some(KnownFn::Vec),
        "mapcat" => Some(KnownFn::Mapcat),
        "repeatedly" => Some(KnownFn::Repeatedly),
        "+" => Some(KnownFn::Add),
        "-" => Some(KnownFn::Sub),
        "*" => Some(KnownFn::Mul),
        "/" => Some(KnownFn::Div),
        "rem" => Some(KnownFn::Rem),
        "mod" => Some(KnownFn::Mod),
        "unchecked-add" | "unchecked-add-int" => Some(KnownFn::UncheckedAdd),
        "unchecked-subtract" | "unchecked-subtract-int" => Some(KnownFn::UncheckedSub),
        "unchecked-multiply" | "unchecked-multiply-int" => Some(KnownFn::UncheckedMul),
        "aget" => Some(KnownFn::Aget),
        "aset" => Some(KnownFn::Aset),
        "alength" => Some(KnownFn::Alength),
        "=" => Some(KnownFn::Eq),
        "case=" => Some(KnownFn::CaseEq),
        "<" => Some(KnownFn::Lt),
        ">" => Some(KnownFn::Gt),
        "<=" => Some(KnownFn::Lte),
        ">=" => Some(KnownFn::Gte),
        "nil?" => Some(KnownFn::IsNil),
        "seq?" => Some(KnownFn::IsSeq),
        "vector?" => Some(KnownFn::IsVector),
        "map?" => Some(KnownFn::IsMap),
        "identical?" => Some(KnownFn::Identical),
        "str" => Some(KnownFn::Str),
        "deref" => Some(KnownFn::Deref),
        "println" => Some(KnownFn::Println),
        "pr" => Some(KnownFn::Pr),
        "apply" => Some(KnownFn::Apply),
        "reduce" => Some(KnownFn::Reduce2), // dispatched by arity in lower_call
        "map" => Some(KnownFn::Map),
        "filter" => Some(KnownFn::Filter),
        "mapv" => Some(KnownFn::Mapv),
        "filterv" => Some(KnownFn::Filterv),
        "some" => Some(KnownFn::Some),
        "every?" => Some(KnownFn::Every),
        "into" => Some(KnownFn::Into), // dispatched by arity
        "concat" => Some(KnownFn::Concat),
        "range" => Some(KnownFn::Range1), // dispatched by arity
        "take" => Some(KnownFn::Take),
        "drop" => Some(KnownFn::Drop),
        "reverse" => Some(KnownFn::Reverse),
        "sort" => Some(KnownFn::Sort),
        "sort-by" => Some(KnownFn::SortBy),
        "keys" => Some(KnownFn::Keys),
        "vals" => Some(KnownFn::Vals),
        "merge" => Some(KnownFn::Merge),
        "update" => Some(KnownFn::Update),
        "get-in" => Some(KnownFn::GetIn),
        "assoc-in" => Some(KnownFn::AssocIn),
        "number?" => Some(KnownFn::IsNumber),
        "string?" => Some(KnownFn::IsString),
        "keyword?" => Some(KnownFn::IsKeyword),
        "symbol?" => Some(KnownFn::IsSymbol),
        "boolean?" => Some(KnownFn::IsBool),
        "int?" => Some(KnownFn::IsInt),
        "prn" => Some(KnownFn::Prn),
        "print" => Some(KnownFn::Print),
        "atom" => Some(KnownFn::Atom),
        "reset!" => Some(KnownFn::AtomReset),
        "swap!" => Some(KnownFn::AtomSwap),
        "group-by" => Some(KnownFn::GroupBy),
        "partition" => Some(KnownFn::Partition2), // dispatched by arity
        "frequencies" => Some(KnownFn::Frequencies),
        "keep" => Some(KnownFn::Keep),
        "remove" => Some(KnownFn::Remove),
        "map-indexed" => Some(KnownFn::MapIndexed),
        "zipmap" => Some(KnownFn::Zipmap),
        "juxt" => Some(KnownFn::Juxt),
        "comp" => Some(KnownFn::Comp),
        "partial" => Some(KnownFn::Partial),
        "complement" => Some(KnownFn::Complement),
        _ => None,
    }
}
