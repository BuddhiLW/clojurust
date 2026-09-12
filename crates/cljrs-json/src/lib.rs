use std::sync::Arc;

use cljrs_gc::GcPtr;
use cljrs_interop::{Registry, wrap_fn1};
use cljrs_runtime::env::env::GlobalEnv;
use cljrs_value::keyword::Keyword;
use cljrs_value::value::MapValue;
use cljrs_value::{PersistentVector, Value};
use serde_json::{Map as JsonMap, Number, Value as Json};

/// Registered under the JVM library's own name so a `.cljrs` source can
/// `(:require [clojure.data.json :as json])` exactly as the `.cljw` one does.
pub const NS: &str = "clojure.data.json";

/// Register the `clojure.data.json` namespace into `globals`.
///
/// Idempotent: the namespace is built only on the first call.
pub fn init(globals: &Arc<GlobalEnv>) {
    if globals.is_loaded(NS) {
        return;
    }
    globals.get_or_create_ns(NS);
    globals.refer_core(NS);
    let mut registry = Registry::for_require(globals.clone());
    register(&mut registry);
}

// ── Clojure -> JSON ─────────────────────────────────────────────────────────

/// The JSON object key a Clojure map key becomes.
///
/// JSON keys are strings, so a keyword contributes its name (`:id` -> "id",
/// `:a/b` -> "a/b"). A key of any other type is rendered the way `str` would
/// render it rather than refused: a map that round-trips through JSON at all
/// is worth more here than one that refuses at the edge.
fn key_string(k: &Value) -> Result<String, String> {
    Ok(match k {
        Value::Keyword(kw) => kw.get().full_name(),
        Value::Symbol(s) => s.get().full_name(),
        Value::Str(s) => s.get().to_string(),
        Value::Long(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Char(c) => c.to_string(),
        Value::Nil => "null".to_string(),
        other => return Err(format!("cannot use {} as a JSON key", other.type_name())),
    })
}

fn number(n: f64) -> Result<Json, String> {
    Number::from_f64(n)
        .map(Json::Number)
        .ok_or_else(|| format!("{n} has no JSON representation"))
}

/// Convert a Clojure value into `serde_json`'s tree.
///
/// Every sequential collection becomes an array and every map becomes an
/// object. NaN and Infinity are REFUSED: JSON cannot spell them, and emitting
/// `null` in their place would silently turn a broken number into a missing
/// one.
pub fn to_json(v: &Value) -> Result<Json, String> {
    Ok(match v {
        Value::Nil => Json::Null,
        Value::Bool(b) => Json::Bool(*b),
        Value::Long(n) => Json::Number(Number::from(*n)),
        Value::Double(d) => number(*d)?,
        Value::Str(s) => Json::String(s.get().to_string()),
        Value::Char(c) => Json::String(c.to_string()),
        Value::Keyword(k) => Json::String(k.get().full_name()),
        Value::Symbol(s) => Json::String(s.get().full_name()),
        Value::Uuid(u) => Json::String(uuid_string(*u)),

        Value::Vector(vec) => {
            let mut out = Vec::with_capacity(vec.get().count());
            for e in vec.get().iter() {
                out.push(to_json(e)?);
            }
            Json::Array(out)
        }
        Value::List(list) => {
            let mut out = Vec::new();
            for e in list.get().iter() {
                out.push(to_json(e)?);
            }
            Json::Array(out)
        }
        Value::Set(set) => {
            let mut out = Vec::new();
            for e in set.iter() {
                out.push(to_json(e)?);
            }
            Json::Array(out)
        }
        Value::Map(m) => {
            let mut obj = JsonMap::new();
            for (k, val) in m.iter() {
                obj.insert(key_string(k)?, to_json(val)?);
            }
            Json::Object(obj)
        }

        other => {
            return Err(format!("no JSON representation for {}", other.type_name()));
        }
    })
}

fn uuid_string(u: u128) -> String {
    let b = u.to_be_bytes();
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

// ── JSON -> Clojure ─────────────────────────────────────────────────────────

/// Convert `serde_json`'s tree into a Clojure value.
///
/// Object keys become STRINGS, matching `clojure.data.json`'s default and the
/// `.cljw` host primitive this mirrors — callers written against either read
/// them with `(get m "key")`. Pass `keywordize` to get keyword keys instead.
///
/// A JSON integer becomes a Long and any other number a Double.
pub fn from_json(j: &Json, keywordize: bool) -> Value {
    match j {
        Json::Null => Value::Nil,
        Json::Bool(b) => Value::Bool(*b),
        Json::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Long(i)
            } else {
                Value::Double(n.as_f64().unwrap_or(f64::NAN))
            }
        }
        Json::String(s) => Value::string(s.clone()),
        Json::Array(items) => {
            let mut v = PersistentVector::empty();
            for item in items {
                v = v.conj(from_json(item, keywordize));
            }
            Value::Vector(GcPtr::new(v))
        }
        Json::Object(obj) => {
            let pairs: Vec<(Value, Value)> = obj
                .iter()
                .map(|(k, val)| {
                    let key = if keywordize {
                        Value::keyword(Keyword::parse(k))
                    } else {
                        Value::string(k.clone())
                    };
                    (key, from_json(val, keywordize))
                })
                .collect();
            Value::Map(MapValue::from_pairs(pairs))
        }
    }
}

// ── the namespace ───────────────────────────────────────────────────────────

pub fn register(registry: &mut Registry) {
    registry.define(
        "clojure.data.json/write-str",
        wrap_fn1(
            "clojure.data.json/write-str",
            |v: Value| -> Result<String, String> {
                serde_json::to_string(&to_json(&v)?).map_err(|e| e.to_string())
            },
        ),
    );

    registry.define(
        "clojure.data.json/read-str",
        wrap_fn1(
            "clojure.data.json/read-str",
            |s: String| -> Result<Value, String> {
                let j: Json = serde_json::from_str(&s).map_err(|e| e.to_string())?;
                Ok(from_json(&j, false))
            },
        ),
    );

    registry.define(
        "clojure.data.json/read-str-keywordize",
        wrap_fn1(
            "clojure.data.json/read-str-keywordize",
            |s: String| -> Result<Value, String> {
                let j: Json = serde_json::from_str(&s).map_err(|e| e.to_string())?;
                Ok(from_json(&j, true))
            },
        ),
    );

    registry.env().mark_loaded(NS);
}

/// # Safety
/// `registry` must be a valid, non-null `*mut Registry` and must remain
/// uniquely borrowed for the duration of the call. The cljrs CLI satisfies
/// both: it allocates the `Registry` on its stack and hands the only pointer
/// to it across the FFI boundary.
#[cfg(feature = "dylib-init")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cljrs_init(registry: *mut Registry) {
    register(unsafe { &mut *registry });
}
