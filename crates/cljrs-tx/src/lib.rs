//! Tree-walker-only runtime for pure, isolated transaction functions.

#![cfg(feature = "no-gc")]

use std::panic::{AssertUnwindSafe, catch_unwind};

use cljrs_env::env::Env;
use cljrs_env::error::EvalError;
use cljrs_reader::{Form, Parser};
use cljrs_value::clone::{CloneError, SerializedValue, deserialize, serialize};

/// A parsed transaction function expression.
///
/// The expression must evaluate to a callable value, normally `(fn [db arg]
/// ...)`. Parsing is intentionally outside the invocation arena: installed
/// transaction code is treated as immutable program data rather than working
/// memory charged on every call.
#[derive(Clone, Debug)]
pub struct TxProgram {
    form: Form,
}

impl TxProgram {
    pub fn parse(source: &str) -> Result<Self, TxError> {
        let mut parser = Parser::new(source.to_string(), "<transaction-function>".to_string());
        let form = parser
            .parse_one()
            .map_err(|error| TxError::Read(Box::new(error)))?
            .ok_or(TxError::EmptyProgram)?;
        if parser
            .parse_one()
            .map_err(|error| TxError::Read(Box::new(error)))?
            .is_some()
        {
            return Err(TxError::MultipleForms);
        }
        Ok(Self { form })
    }

    pub fn form(&self) -> &Form {
        &self.form
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TxLimits {
    /// Managed allocation budget for the invocation arena.
    pub memory_bytes: usize,
    /// Cooperative tree-walker execution credits.
    pub gas: u64,
}

impl Default for TxLimits {
    fn default() -> Self {
        Self {
            memory_bytes: 16 * 1024 * 1024,
            gas: 1_000_000,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TxError {
    #[error("transaction function source is empty")]
    EmptyProgram,
    #[error("transaction function source must contain exactly one form")]
    MultipleForms,
    #[error("transaction memory limit must be at least 64 bytes")]
    InvalidMemoryLimit,
    #[error("transaction function read error: {0}")]
    Read(Box<cljrs_types::error::CljxError>),
    #[error("transaction input is not isolated pure data: {0}")]
    InvalidInput(&'static str),
    #[error("transaction result is not isolated pure data: {0}")]
    InvalidResult(&'static str),
    #[error("transaction evaluation failed: {0}")]
    Evaluation(String),
    #[error("transaction gas exhausted")]
    GasExhausted,
    #[error("transaction attempted a forbidden effect: {0}")]
    ForbiddenEffect(String),
    #[error(
        "transaction managed-memory budget exhausted (limit {limit} bytes, used {used}, requested {requested})"
    )]
    MemoryExhausted {
        limit: usize,
        used: usize,
        requested: usize,
    },
    #[error("transaction result cannot cross the isolation boundary: {0}")]
    Clone(#[from] CloneError),
    #[error("transaction runtime panicked")]
    Panic,
}

/// Execute `program` in a fresh GC-less arena and return an owned data value.
///
/// Inputs are structured-cloned into the invocation. The environment, inputs,
/// closures, lazy sequences, local mutable cells, and all intermediate values
/// are destroyed together when execution finishes. The result is cloned out
/// before the arena is freed.
pub fn execute(
    program: &TxProgram,
    args: Vec<SerializedValue>,
    limits: TxLimits,
) -> Result<SerializedValue, TxError> {
    if limits.memory_bytes < 64 {
        return Err(TxError::InvalidMemoryLimit);
    }
    for arg in &args {
        validate_pure_data(arg).map_err(TxError::InvalidInput)?;
    }

    match catch_unwind(AssertUnwindSafe(|| execute_inner(program, args, limits))) {
        Ok(result) => result,
        Err(payload) => match payload.downcast_ref::<cljrs_gc::region::RegionLimitExceeded>() {
            Some(exhausted) => Err(TxError::MemoryExhausted {
                limit: exhausted.limit,
                used: exhausted.used,
                requested: exhausted.requested,
            }),
            None => Err(TxError::Panic),
        },
    }
}

fn execute_inner(
    program: &TxProgram,
    args: Vec<SerializedValue>,
    limits: TxLimits,
) -> Result<SerializedValue, TxError> {
    let invocation = cljrs_gc::alloc_ctx::InvocationGuard::new(limits.memory_bytes);

    // Bootstrap is trusted but is deliberately constructed inside the arena so
    // the complete namespace/Var/function environment dies with the call.
    let globals = cljrs_interp::standard_env_minimal(None, None, None);
    let mut env = Env::new(globals, "user");
    let args: Vec<_> = args.into_iter().map(deserialize).collect();

    let _policy = cljrs_env::policy::TransactionPolicyGuard::install();
    let meter = cljrs_env::gas::GasMeter::new(limits.gas);
    let _gas = cljrs_env::gas::GasGuard::install(meter);

    let function = env.eval(program.form()).map_err(map_eval_error)?;
    let result =
        cljrs_env::apply::apply_value(&function, args, &mut env).map_err(map_eval_error)?;
    let result = serialize(&result)?;
    validate_pure_data(&result).map_err(TxError::InvalidResult)?;

    // Make the lifetime ordering explicit: all GcPtrs disappear before the
    // invocation arena is reset by its guard.
    drop(env);
    drop(function);
    let _managed_bytes = invocation.accounted_bytes();
    Ok(result)
}

fn map_eval_error(error: EvalError) -> TxError {
    match error {
        EvalError::GasExhausted => TxError::GasExhausted,
        EvalError::ForbiddenEffect(operation) => TxError::ForbiddenEffect(operation),
        other => TxError::Evaluation(other.to_string()),
    }
}

fn validate_pure_data(value: &SerializedValue) -> Result<(), &'static str> {
    use SerializedValue::*;
    match value {
        SharedAtom(_) => Err("shared atom"),
        ByteBlob(_) => Err("shared byte blob"),
        Var { .. } => Err("Var"),
        Error(_) => Err("error object"),
        List(items) | Vector(items) | HashSet(items) | SortedSet(items) | Queue(items)
        | ObjectArray(items) => items.iter().try_for_each(validate_pure_data),
        ArrayMap(pairs) | HashMap(pairs) | SortedMap(pairs) => {
            pairs.iter().try_for_each(|(key, value)| {
                validate_pure_data(key).and_then(|()| validate_pure_data(value))
            })
        }
        Cons { head, tail } => validate_pure_data(head).and_then(|()| validate_pure_data(tail)),
        TypeInstance { fields, .. } => fields.iter().try_for_each(|(key, value)| {
            validate_pure_data(key).and_then(|()| validate_pure_data(value))
        }),
        WithMeta { value, meta } => {
            validate_pure_data(value).and_then(|()| validate_pure_data(meta))
        }
        Reduced(inner) => validate_pure_data(inner),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executes_pure_tree_walked_function() {
        let program = TxProgram::parse("(fn [x] (assoc x :seen true))").unwrap();
        let input = SerializedValue::ArrayMap(vec![(
            SerializedValue::Keyword {
                namespace: None,
                name: "id".into(),
            },
            SerializedValue::Long(7),
        )]);
        let result = execute(&program, vec![input], TxLimits::default()).unwrap();
        assert!(matches!(result, SerializedValue::ArrayMap(_)));
    }

    #[test]
    fn permits_internal_closure_that_does_not_escape() {
        let program = TxProgram::parse("(fn [x] (let [f (fn [y] (+ x y))] (f 2)))").unwrap();
        let result = execute(
            &program,
            vec![SerializedValue::Long(40)],
            TxLimits::default(),
        )
        .unwrap();
        assert!(matches!(result, SerializedValue::Long(42)));
    }

    #[test]
    fn rejects_effectful_builtin() {
        let program = TxProgram::parse("(fn [] (spit \"/tmp/cljrs-tx-test\" \"no\"))").unwrap();
        let error = execute(&program, vec![], TxLimits::default()).unwrap_err();
        assert!(matches!(error, TxError::ForbiddenEffect(name) if name == "spit"));
    }

    #[test]
    fn enforces_gas_limit() {
        let program = TxProgram::parse("(fn [] (loop [x 0] (recur (inc x))))").unwrap();
        let error = execute(
            &program,
            vec![],
            TxLimits {
                gas: 100,
                ..TxLimits::default()
            },
        )
        .unwrap_err();
        assert!(matches!(error, TxError::GasExhausted));
    }

    #[test]
    fn enforces_managed_memory_limit() {
        let program = TxProgram::parse("(fn [] (vec (range 100000)))").unwrap();
        let error = execute(
            &program,
            vec![],
            TxLimits {
                memory_bytes: 256 * 1024,
                gas: 1_000_000,
            },
        )
        .unwrap_err();
        assert!(matches!(error, TxError::MemoryExhausted { .. }));
    }

    #[test]
    fn syntax_quote_gensyms_are_deterministic_per_invocation() {
        let program = TxProgram::parse("(fn [] `local#)").unwrap();
        let first = execute(&program, vec![], TxLimits::default()).unwrap();
        let second = execute(&program, vec![], TxLimits::default()).unwrap();
        let symbol_name = |value: &SerializedValue| match value {
            SerializedValue::Symbol { name, .. } => name.to_string(),
            other => panic!("expected symbol, got {other:?}"),
        };
        assert_eq!(symbol_name(&first), symbol_name(&second));
    }
}
