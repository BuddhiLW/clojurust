//! IFn on a persistent vector: `([:a :b] 1)` is `(nth [:a :b] 1)`.
//!
//! Kept out of `apply.rs` so the dispatcher only routes here.

use cljrs_value::{PersistentVector, Value};

use crate::env::error::{EvalError, EvalResult};

/// Invoke vector `v` with `args`: one integer index; out of range throws,
/// as Clojure's `APersistentVector.invoke` does.
pub fn invoke(v: &PersistentVector, args: &[Value]) -> EvalResult<Value> {
    if args.len() != 1 {
        return Err(EvalError::Arity {
            name: "vector".into(),
            expected: "1".into(),
            got: args.len(),
        });
    }
    let idx = match args[0].unwrap_meta() {
        Value::Long(i) => *i,
        _ => {
            return Err(EvalError::Runtime(
                "Key must be integer when invoking a vector".into(),
            ));
        }
    };
    usize::try_from(idx)
        .ok()
        .and_then(|i| v.nth(i).cloned())
        .ok_or_else(|| {
            EvalError::Runtime(format!(
                "Index {idx} out of bounds for vector of length {}",
                v.count()
            ))
        })
}
