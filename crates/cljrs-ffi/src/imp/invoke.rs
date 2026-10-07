//! The two call shapes and the decoding of a return register.

use std::ffi::{CStr, c_char};

use cljrs_value::Value;

use super::sig::{CType, Packed};

type IntShape = unsafe extern "C" fn(
    usize,
    usize,
    usize,
    usize,
    usize,
    usize,
    f64,
    f64,
    f64,
    f64,
    f64,
    f64,
    f64,
    f64,
) -> usize;

type DoubleShape = unsafe extern "C" fn(
    usize,
    usize,
    usize,
    usize,
    usize,
    usize,
    f64,
    f64,
    f64,
    f64,
    f64,
    f64,
    f64,
    f64,
) -> f64;

/// Call the C function at `addr` with `p`'s registers and decode its result.
///
/// # Safety
/// `addr` must be the address of a live, non-variadic C function whose
/// parameters are exactly the declared signature that produced `p`.
pub(crate) unsafe fn call(addr: usize, p: &Packed, ret: CType) -> Value {
    let i = &p.ints;
    let d = &p.dbls;
    if ret == CType::Double {
        let f: DoubleShape = unsafe { std::mem::transmute(addr) };
        let r = unsafe {
            f(
                i[0], i[1], i[2], i[3], i[4], i[5], d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7],
            )
        };
        return Value::Double(r);
    }
    let f: IntShape = unsafe { std::mem::transmute(addr) };
    let r = unsafe {
        f(
            i[0], i[1], i[2], i[3], i[4], i[5], d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7],
        )
    };
    match ret {
        CType::Void => Value::Nil,
        CType::Int => Value::Long(r as u32 as i32 as i64),
        CType::Long | CType::Pointer => Value::Long(r as i64),
        CType::Str => unsafe { c_string(r) }.unwrap_or(Value::Nil),
        CType::Double | CType::Bytes => unreachable!("handled above / refused at resolve"),
    }
}

/// Copy the NUL-terminated string at `addr`; `None` for NULL.
///
/// # Safety
/// `addr` must be 0 or point at a readable NUL-terminated byte string.
pub(crate) unsafe fn c_string(addr: usize) -> Option<Value> {
    if addr == 0 {
        return None;
    }
    let s = unsafe { CStr::from_ptr(addr as *const c_char) };
    Some(Value::string(s.to_string_lossy().into_owned()))
}

/// Copy `n` bytes at `addr`.
///
/// # Safety
/// `addr` must point at `n` readable bytes.
pub(crate) unsafe fn c_bytes(addr: usize, n: usize) -> Vec<i8> {
    if n == 0 {
        return Vec::new();
    }
    let s = unsafe { std::slice::from_raw_parts(addr as *const i8, n) };
    s.to_vec()
}
