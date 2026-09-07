//! `Value` → Rust coercions shared by every drawing entry point.
//!
//! Numeric arguments accept `Long` or `Double` interchangeably; anything else
//! is an error naming the type that was passed. Options maps accept `Nil` as
//! "no options", so every `…!` function can be called with or without a
//! trailing map.

use cljrs_value::{Keyword, MapValue, Value};

/// A simple (unqualified) keyword value, e.g. `:color`.
pub fn kw(name: &str) -> Value {
    Value::keyword(Keyword::simple(name))
}

/// Coerce a numeric `Value` to `f32`.
pub fn as_f32(v: &Value) -> Result<f32, String> {
    match v {
        Value::Long(n) => Ok(*n as f32),
        Value::Double(d) => Ok(*d as f32),
        other => Err(format!("expected a number, got {}", other.type_name())),
    }
}

/// Coerce a numeric `Value` to `i32`, truncating a `Double`.
pub fn as_i32(v: &Value) -> Result<i32, String> {
    match v {
        Value::Long(n) => Ok(*n as i32),
        Value::Double(d) => Ok(*d as i32),
        other => Err(format!("expected an integer, got {}", other.type_name())),
    }
}

/// Coerce a numeric `Value` to `u32`, rejecting negatives.
pub fn as_u32(v: &Value) -> Result<u32, String> {
    let n = as_i32(v)?;
    u32::try_from(n).map_err(|_| format!("expected a non-negative integer, got {n}"))
}

/// Normalize an options argument: a map stays a map, `Nil` becomes the empty
/// map, anything else is an error.
pub fn opts_map(v: &Value) -> Result<MapValue, String> {
    match v {
        Value::Nil => Ok(MapValue::empty()),
        Value::Map(m) => Ok(m.clone()),
        other => Err(format!(
            "expected an options map, got {}",
            other.type_name()
        )),
    }
}

/// Look up `:key` in an options map.
pub fn opt(m: &MapValue, key: &str) -> Option<Value> {
    m.get(&kw(key))
}

/// Look up `:key` as a number.
pub fn opt_f32(m: &MapValue, key: &str) -> Result<Option<f32>, String> {
    match opt(m, key) {
        None | Some(Value::Nil) => Ok(None),
        Some(v) => as_f32(&v).map(Some),
    }
}

/// Look up `:key` as a boolean, treating `nil` as absent (not `false`).
pub fn opt_bool(m: &MapValue, key: &str) -> Option<bool> {
    match opt(m, key) {
        Some(Value::Bool(b)) => Some(b),
        Some(Value::Nil) | None => None,
        // Clojure truthiness: everything that is neither nil nor false is true.
        Some(_) => Some(true),
    }
}

/// Look up `:key` as a keyword or string, returning its bare name.
///
/// Both spellings are accepted so `{:line-cap :round}` and
/// `{:line-cap "round"}` mean the same thing.
pub fn opt_name(m: &MapValue, key: &str) -> Result<Option<String>, String> {
    match opt(m, key) {
        None | Some(Value::Nil) => Ok(None),
        Some(Value::Keyword(k)) => Ok(Some(k.get().name.to_string())),
        Some(Value::Str(s)) => Ok(Some(s.get().clone())),
        Some(other) => Err(format!(
            "expected a keyword or string for :{key}, got {}",
            other.type_name()
        )),
    }
}

/// Read a vector of `n` numbers, e.g. `[x y]` or `[sx sy]`.
pub fn as_coords(v: &Value, n: usize, what: &str) -> Result<Vec<f32>, String> {
    match v {
        Value::Vector(vec) => {
            let vec = vec.get();
            if vec.count() != n {
                return Err(format!(
                    "{what} must be a vector of {n} numbers, got {} element(s)",
                    vec.count()
                ));
            }
            vec.iter().map(as_f32).collect()
        }
        other => Err(format!(
            "{what} must be a vector of {n} numbers, got {}",
            other.type_name()
        )),
    }
}

/// Read a vector of `[x y]` points, e.g. the vertices of a polygon.
pub fn as_points(v: &Value) -> Result<Vec<(f32, f32)>, String> {
    match v {
        Value::Vector(vec) => vec
            .get()
            .iter()
            .map(|p| {
                let xy = as_coords(p, 2, "a point")?;
                Ok((xy[0], xy[1]))
            })
            .collect(),
        other => Err(format!(
            "expected a vector of [x y] points, got {}",
            other.type_name()
        )),
    }
}

/// Read a vector of numbers of any length, e.g. a dash pattern.
pub fn as_f32_vec(v: &Value, what: &str) -> Result<Vec<f32>, String> {
    match v {
        Value::Vector(vec) => vec.get().iter().map(as_f32).collect(),
        other => Err(format!(
            "{what} must be a vector of numbers, got {}",
            other.type_name()
        )),
    }
}
