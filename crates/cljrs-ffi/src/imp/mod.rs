//! The namespace's functions, on targets that have it.

mod error;
mod invoke;
mod library;
mod sig;

use std::sync::{Arc, Mutex};

use cljrs_gc::GcPtr;
use cljrs_runtime::env::env::GlobalEnv;
use cljrs_value::{Arity, NativeFn, NativeObjectBox, Value, ValueResult};

use crate::NS;
use error::{arg_type_error, wrong_type};
use library::{LibHandle, LibInner};
use sig::Signature;

type Builtin = fn(&[Value]) -> ValueResult<Value>;

pub(crate) fn register(globals: &Arc<GlobalEnv>) {
    let fns: [(&str, Arity, Builtin); 7] = [
        ("open", Arity::Fixed(1), builtin_open),
        ("close", Arity::Fixed(1), builtin_close),
        ("sym", Arity::Fixed(2), builtin_sym),
        ("function", Arity::Fixed(4), builtin_function),
        ("call", Arity::Variadic { min: 4 }, builtin_call),
        ("string", Arity::Fixed(1), builtin_string),
        ("bytes", Arity::Fixed(2), builtin_bytes),
    ];
    for (name, arity, func) in fns {
        // Qualified, so the transaction policy can deny the whole namespace
        // by prefix.
        let nf = NativeFn::new(format!("{NS}/{name}"), arity, func);
        globals.intern(NS, Arc::from(name), Value::NativeFunction(GcPtr::new(nf)));
    }
}

fn lib_of(v: &Value) -> ValueResult<Arc<LibInner>> {
    if let Value::NativeObject(obj) = v
        && let Some(h) = obj.get().downcast_ref::<LibHandle>()
    {
        return Ok(h.0.clone());
    }
    Err(wrong_type("an ffi library handle", v))
}

fn name_of(v: &Value) -> ValueResult<String> {
    match v {
        Value::Str(s) => Ok(s.get().to_string()),
        Value::Symbol(s) => Ok(s.get().name.to_string()),
        Value::Keyword(k) => Ok(k.get().name.to_string()),
        other => Err(wrong_type("a string", other)),
    }
}

/// An address argument: an integer, or nil for NULL.
fn address_of(index: usize, v: &Value) -> ValueResult<usize> {
    match v {
        Value::Nil => Ok(0),
        Value::Long(n) => Ok(*n as usize),
        other => Err(arg_type_error(index, "pointer", other)),
    }
}

fn builtin_open(args: &[Value]) -> ValueResult<Value> {
    let path = name_of(&args[0])?;
    let inner = LibInner::open(&path)?;
    Ok(Value::NativeObject(GcPtr::new(NativeObjectBox::new(
        LibHandle(inner),
    ))))
}

fn builtin_close(args: &[Value]) -> ValueResult<Value> {
    lib_of(&args[0])?.close();
    Ok(Value::Nil)
}

fn builtin_sym(args: &[Value]) -> ValueResult<Value> {
    let lib = lib_of(&args[0])?;
    let name = name_of(&args[1])?;
    Ok(Value::Long(lib.symbol(&name)? as i64))
}

/// Resolve the signature and the symbol once, and close over both.
fn bind(args: &[Value]) -> ValueResult<NativeFn> {
    let lib = lib_of(&args[0])?;
    let name = name_of(&args[1])?;
    let sig = Signature::resolve(&args[2], &args[3])?;
    let addr = lib.symbol(&name)?;
    let fn_name = format!("{NS}/function:{name}");
    Ok(NativeFn::with_closure(
        fn_name,
        Arity::Variadic { min: 0 },
        move |vals: &[Value]| {
            // The read guard keeps `close` from unmapping the library while
            // the call is in flight.
            let _open = lib.guard()?;
            let packed = sig.pack(vals)?;
            // SAFETY: `addr` was resolved from this still-open library, and
            // `packed` matches the signature the caller declared for it.
            Ok(unsafe { invoke::call(addr, &packed, sig.ret) })
        },
    ))
}

fn builtin_function(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::NativeFunction(GcPtr::new(bind(args)?)))
}

fn builtin_call(args: &[Value]) -> ValueResult<Value> {
    let f = bind(&args[..4])?;
    (f.func)(&args[4..])
}

fn builtin_string(args: &[Value]) -> ValueResult<Value> {
    let addr = address_of(0, &args[0])?;
    // SAFETY: the caller asserts `addr` is NULL or a NUL-terminated string.
    Ok(unsafe { invoke::c_string(addr) }.unwrap_or(Value::Nil))
}

fn builtin_bytes(args: &[Value]) -> ValueResult<Value> {
    let addr = address_of(0, &args[0])?;
    let n = match &args[1] {
        Value::Long(n) if *n >= 0 => *n as usize,
        other => return Err(arg_type_error(1, "long", other)),
    };
    if addr == 0 && n > 0 {
        return Err(arg_type_error(0, "pointer", &args[0]));
    }
    // SAFETY: the caller asserts `addr` points at `n` readable bytes.
    let data = unsafe { invoke::c_bytes(addr, n) };
    Ok(Value::ByteArray(GcPtr::new(Mutex::new(data))))
}
