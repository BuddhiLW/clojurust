//! JVM numeric statics that portable Clojure libraries call directly.
//!
//! `clojure.test.check` (and through it `malli.generator`) is written against
//! `Long`, `Double`, `Math` and `Character` statics. Each one here has an exact
//! Rust counterpart; they are interned into `clojure.core` under their
//! `Class/member` spelling, the same way `Math/abs` and `Math/PI` are.

use std::sync::Arc;

use crate::builtins::util::numeric_as_i64;
use crate::env::env::GlobalEnv;
use cljrs_gc::GcPtr;
use cljrs_value::{Arity, NativeFn, Value, ValueError, ValueResult};

fn as_f64(v: &Value) -> ValueResult<f64> {
    match v {
        Value::Double(d) => Ok(*d),
        Value::Long(n) => Ok(*n as f64),
        other => Err(ValueError::Other(format!(
            "expected a number, got {}",
            other.type_name()
        ))),
    }
}

fn long_bit_count(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Long(numeric_as_i64(&args[0])?.count_ones() as i64))
}
fn long_reverse(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Long(numeric_as_i64(&args[0])?.reverse_bits()))
}
fn long_leading_zeros(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Long(numeric_as_i64(&args[0])?.leading_zeros() as i64))
}
fn long_trailing_zeros(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Long(
        numeric_as_i64(&args[0])?.trailing_zeros() as i64
    ))
}
fn double_is_nan(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(as_f64(&args[0])?.is_nan()))
}
fn double_is_infinite(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(as_f64(&args[0])?.is_infinite()))
}
fn double_to_long_bits(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Long(as_f64(&args[0])?.to_bits() as i64))
}
fn long_bits_to_double(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Double(f64::from_bits(
        numeric_as_i64(&args[0])? as u64
    )))
}

/// `Math/scalb` — `x * 2^exp`, stepping so every factor stays a normal double.
fn scalb(args: &[Value]) -> ValueResult<Value> {
    let mut r = as_f64(&args[0])?;
    let mut e = numeric_as_i64(&args[1])?.clamp(-2200, 2200);
    while e > 1000 {
        r *= 2f64.powi(1000);
        e -= 1000;
    }
    while e < -1000 {
        r *= 2f64.powi(-1000);
        e += 1000;
    }
    Ok(Value::Double(r * 2f64.powi(e as i32)))
}

/// `Math/getExponent` — the unbiased binary exponent of a double.
fn get_exponent(args: &[Value]) -> ValueResult<Value> {
    let bits = as_f64(&args[0])?.to_bits();
    Ok(Value::Long((((bits >> 52) & 0x7ff) as i64) - 1023))
}

fn char_is_digit(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Char(c) => Ok(Value::Bool(c.is_ascii_digit())),
        v => Err(ValueError::Other(format!(
            "Character/isDigit expects a char, got {}",
            v.type_name()
        ))),
    }
}

type Static = (&'static str, Arity, fn(&[Value]) -> ValueResult<Value>);

const FNS: &[Static] = &[
    ("Long/bitCount", Arity::Fixed(1), long_bit_count),
    ("Long/reverse", Arity::Fixed(1), long_reverse),
    (
        "Long/numberOfLeadingZeros",
        Arity::Fixed(1),
        long_leading_zeros,
    ),
    (
        "Long/numberOfTrailingZeros",
        Arity::Fixed(1),
        long_trailing_zeros,
    ),
    ("Double/isNaN", Arity::Fixed(1), double_is_nan),
    ("Double/isInfinite", Arity::Fixed(1), double_is_infinite),
    (
        "Double/doubleToLongBits",
        Arity::Fixed(1),
        double_to_long_bits,
    ),
    (
        "Double/longBitsToDouble",
        Arity::Fixed(1),
        long_bits_to_double,
    ),
    ("Math/scalb", Arity::Fixed(2), scalb),
    ("Math/getExponent", Arity::Fixed(1), get_exponent),
    ("Character/isDigit", Arity::Fixed(1), char_is_digit),
];

const CONSTS: &[(&str, Value)] = &[
    ("Long/MIN_VALUE", Value::Long(i64::MIN)),
    ("Integer/MAX_VALUE", Value::Long(i32::MAX as i64)),
    ("Integer/MIN_VALUE", Value::Long(i32::MIN as i64)),
    ("Short/MAX_VALUE", Value::Long(i16::MAX as i64)),
    ("Short/MIN_VALUE", Value::Long(i16::MIN as i64)),
    ("Byte/MAX_VALUE", Value::Long(i8::MAX as i64)),
    ("Byte/MIN_VALUE", Value::Long(i8::MIN as i64)),
    ("Double/POSITIVE_INFINITY", Value::Double(f64::INFINITY)),
    ("Double/NEGATIVE_INFINITY", Value::Double(f64::NEG_INFINITY)),
    ("Double/NaN", Value::Double(f64::NAN)),
    ("Double/MAX_VALUE", Value::Double(f64::MAX)),
    ("Double/MIN_VALUE", Value::Double(f64::from_bits(1))),
];

/// Intern every static above into `ns`.
pub fn register(globals: &Arc<GlobalEnv>, ns: &str) {
    for (name, arity, f) in FNS {
        let nf = NativeFn::new(*name, arity.clone(), *f);
        globals.intern(ns, Arc::from(*name), Value::NativeFunction(GcPtr::new(nf)));
    }
    for (name, v) in CONSTS {
        globals.intern(ns, Arc::from(*name), v.clone());
    }
}
