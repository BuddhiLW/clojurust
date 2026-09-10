//! The `defrecord` constructor and `record?`.

use std::sync::Arc;

use cljrs_gc::GcPtr;
use cljrs_value::value::{DatatypeKind, TypeInstance};
use cljrs_value::{MapValue, Value, ValueError, ValueResult};

/// The type tag of a string or symbol.
fn type_tag_of(v: &Value) -> ValueResult<Arc<str>> {
    match v {
        Value::Str(s) => Ok(Arc::from(s.get().as_str())),
        Value::Symbol(s) => Ok(Arc::from(s.get().name.as_ref())),
        v => Err(ValueError::WrongType {
            expected: "string or symbol",
            got: v.type_name().to_string(),
        }),
    }
}

/// The field map of a map or `nil`.
fn fields_of(v: &Value) -> ValueResult<MapValue> {
    match v {
        Value::Map(m) => Ok(m.clone()),
        Value::Nil => Ok(MapValue::empty()),
        v => Err(ValueError::WrongType {
            expected: "map",
            got: v.type_name().to_string(),
        }),
    }
}

/// `(make-record-instance type-tag fields-map)` -> a `defrecord` instance.
pub fn make_record_instance(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::TypeInstance(GcPtr::new(TypeInstance {
        type_tag: type_tag_of(&args[0])?,
        fields: fields_of(&args[1])?,
        // Mutable fields are deftype-only.
        mutable: None,
        kind: DatatypeKind::Record,
    })))
}

/// `(record? x)` -> true only for a `defrecord` instance.
///
/// Reads through metadata: `(record? (with-meta r {:k 1}))` is true, as it is
/// on the JVM, where the metadata wrapper is not part of the type.
pub fn record_q(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(match args[0].unwrap_meta() {
        Value::TypeInstance(ti) => ti.get().kind.is_record(),
        _ => false,
    }))
}
