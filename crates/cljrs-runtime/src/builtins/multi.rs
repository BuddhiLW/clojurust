//! Multimethod predicates.
//!
//! `defmethod` is a macro now, so the checks that used to happen in Rust before
//! the target was evaluated have to be expressible from Clojure. `(type m)`
//! answers `Fn` for a multimethod, so nothing in core could tell one apart.

use cljrs_value::{Value, ValueResult};

/// `(multi-fn? x)`: true for a multimethod, false for anything else.
pub fn multi_fn_q(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(matches!(
        args[0].unwrap_meta(),
        Value::MultiFn(_)
    )))
}
