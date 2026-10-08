//! Special forms written through a `clojure.core` alias.
//!
//! `(:require [clojure.core :as core])` plus `(:refer-clojure :exclude [let])`
//! is how a library defines its own `let` and still binds with `core/let`
//! (clojure.test.check.generators does). On the JVM a special form's
//! namespace qualifier is ignored; here the head is looked up by its name
//! when the qualifier resolves to `clojure.core`.

use crate::env::env::Env;
use crate::interp::eval::is_special_form;

/// The special-form name of `ns/name` when `ns` resolves to `clojure.core`.
pub fn qualified_core_special<'a>(s: &'a str, env: &Env) -> Option<&'a str> {
    let (ns_part, name) = s.split_once('/')?;
    if name.is_empty() || !is_special_form(name) {
        return None;
    }
    (env.resolve_ns_part(ns_part).as_ref() == "clojure.core").then_some(name)
}
