//! The `ex-info` values every `clojure.rust.ffi` failure throws.
//!
//! Each carries `:ffi/error <kind>` plus the kind's own keys, so a caller
//! dispatches on `(:ffi/error (ex-data e))` rather than on message text.

use cljrs_gc::GcPtr;
use cljrs_value::{ExceptionInfo, Keyword, MapValue, Value, ValueError};

pub(crate) fn kw(name: &str) -> Value {
    Value::keyword(Keyword::parse(name))
}

/// A thrown `ex-info` with `message` and data `{:ffi/error kind, extra...}`.
pub(crate) fn ffi_error(kind: &str, message: String, extra: Vec<(&str, Value)>) -> ValueError {
    let mut pairs = Vec::with_capacity(extra.len() + 1);
    pairs.push((kw("ffi/error"), kw(kind)));
    for (k, v) in extra {
        pairs.push((kw(k), v));
    }
    let info = ExceptionInfo::new(
        ValueError::Other(message.clone()),
        message,
        Some(MapValue::from_pairs(pairs)),
        None,
    );
    ValueError::Thrown(Value::Error(GcPtr::new(info)))
}

pub(crate) fn open_error(path: &str, reason: String) -> ValueError {
    ffi_error(
        "open",
        format!("ffi: cannot open {path}: {reason}"),
        vec![
            ("path", Value::string(path.to_string())),
            ("reason", Value::string(reason)),
        ],
    )
}

pub(crate) fn closed_error() -> ValueError {
    ffi_error("closed", "ffi: library is closed".to_string(), vec![])
}

pub(crate) fn symbol_error(name: &str, reason: String) -> ValueError {
    ffi_error(
        "symbol",
        format!("ffi: symbol {name} not found: {reason}"),
        vec![("symbol", Value::string(name.to_string()))],
    )
}

pub(crate) fn signature_error(reason: String, extra: Vec<(&str, Value)>) -> ValueError {
    let mut data = vec![("reason", Value::string(reason.clone()))];
    data.extend(extra);
    ffi_error("signature", format!("ffi: bad signature: {reason}"), data)
}

pub(crate) fn arity_error(expected: usize, got: usize) -> ValueError {
    ffi_error(
        "arity",
        format!("ffi: expected {expected} args, got {got}"),
        vec![
            ("expected", Value::Long(expected as i64)),
            ("got", Value::Long(got as i64)),
        ],
    )
}

pub(crate) fn arg_type_error(index: usize, ty: &str, got: &Value) -> ValueError {
    ffi_error(
        "arg-type",
        format!(
            "ffi: argument {index} must be {ty}, got {}",
            got.type_name()
        ),
        vec![("index", Value::Long(index as i64)), ("type", kw(ty))],
    )
}

/// A plain type error on one of the namespace's own arguments (a handle that
/// is not a handle, a name that is not a string): a caller bug, not an FFI
/// condition, so it carries no `:ffi/error`.
// Library handles are the exception: lib_of reports :ffi/error :arg-type.
pub(crate) fn wrong_type(expected: &'static str, got: &Value) -> ValueError {
    ValueError::WrongType {
        expected,
        got: got.type_name().to_string(),
    }
}

pub(crate) fn with_signature_symbol(error: ValueError, name: &str) -> ValueError {
    match error {
        ValueError::Thrown(Value::Error(info)) => {
            let data = info.get().data().expect("signature errors carry ex-data");
            let reason = data
                .get(&kw("reason"))
                .expect("signature errors carry a reason");
            let mut extra = vec![
                ("symbol", Value::string(name.to_string())),
                ("reason", reason),
            ];
            if let Some(ty) = data.get(&kw("type")) {
                extra.push(("type", ty));
            }
            ffi_error("signature", info.get().message(), extra)
        }
        other => other,
    }
}
