//! C types, signatures resolved once at `function` creation, and the packing
//! of Clojure arguments into the two register files.
//!
//! Integer-class arguments fill `ints` in order, `:double` arguments fill
//! `dbls` in order; see `docs/book/src/rust-interop/c-ffi.md`.

use std::ffi::CString;

use cljrs_value::{Value, ValueResult};

use super::error::{arg_type_error, arity_error, signature_error};

pub(crate) const MAX_INT_ARGS: usize = 6;
pub(crate) const MAX_DOUBLE_ARGS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CType {
    Void,
    Int,
    Long,
    Double,
    Pointer,
    Str,
    Bytes,
}

impl CType {
    pub(crate) fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "void" => CType::Void,
            "int" => CType::Int,
            "long" => CType::Long,
            "double" => CType::Double,
            "pointer" => CType::Pointer,
            "string" => CType::Str,
            "bytes" => CType::Bytes,
            _ => return None,
        })
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            CType::Void => "void",
            CType::Int => "int",
            CType::Long => "long",
            CType::Double => "double",
            CType::Pointer => "pointer",
            CType::Str => "string",
            CType::Bytes => "bytes",
        }
    }
}

/// A checked signature: every type known, both register files within bounds.
#[derive(Clone, Debug)]
pub(crate) struct Signature {
    pub(crate) args: Vec<CType>,
    pub(crate) ret: CType,
}

fn type_name_of(v: &Value) -> Option<String> {
    match v {
        Value::Keyword(k) => Some(k.get().name.to_string()),
        Value::Str(s) => Some(s.get().to_string()),
        _ => None,
    }
}

fn parse_type(v: &Value) -> ValueResult<CType> {
    let unknown = || signature_error(format!("unknown type {v}"), vec![("type", v.clone())]);
    let name = type_name_of(v).ok_or_else(unknown)?;
    CType::parse(&name).ok_or_else(unknown)
}

fn seq_items(v: &Value) -> ValueResult<Vec<Value>> {
    match v {
        Value::Nil => Ok(Vec::new()),
        Value::Vector(vec) => Ok(vec.get().iter().cloned().collect()),
        Value::List(list) => Ok(list.get().iter().cloned().collect()),
        other => Err(signature_error(
            format!("arg-types must be a vector, got {}", other.type_name()),
            vec![],
        )),
    }
}

impl Signature {
    /// Resolve `arg-types` and `ret-type`, refusing anything the two call
    /// shapes cannot express.
    pub(crate) fn resolve(arg_types: &Value, ret_type: &Value) -> ValueResult<Self> {
        let mut args = Vec::new();
        for t in seq_items(arg_types)? {
            let ty = parse_type(&t)?;
            if ty == CType::Void {
                return Err(signature_error(
                    ":void is a return type only".to_string(),
                    vec![("type", t)],
                ));
            }
            args.push(ty);
        }
        let ret = parse_type(ret_type)?;
        if ret == CType::Bytes {
            return Err(signature_error(
                ":bytes is an argument type only".to_string(),
                vec![("type", ret_type.clone())],
            ));
        }
        let doubles = args.iter().filter(|t| **t == CType::Double).count();
        let ints = args.len() - doubles;
        if ints > MAX_INT_ARGS {
            return Err(signature_error(
                format!("{ints} integer-class args, at most {MAX_INT_ARGS}"),
                vec![],
            ));
        }
        if doubles > MAX_DOUBLE_ARGS {
            return Err(signature_error(
                format!("{doubles} :double args, at most {MAX_DOUBLE_ARGS}"),
                vec![],
            ));
        }
        Ok(Signature { args, ret })
    }

    /// Pack `vals` into register slots. The returned `Packed` owns every
    /// temporary C string and byte copy, so they live until it is dropped.
    pub(crate) fn pack(&self, vals: &[Value]) -> ValueResult<Packed> {
        if vals.len() != self.args.len() {
            return Err(arity_error(self.args.len(), vals.len()));
        }
        let mut p = Packed::default();
        let (mut ni, mut nd) = (0, 0);
        for (i, (ty, v)) in self.args.iter().zip(vals).enumerate() {
            if *ty == CType::Double {
                p.dbls[nd] = to_double(i, v)?;
                nd += 1;
            } else {
                p.ints[ni] = p.int_slot(i, *ty, v)?;
                ni += 1;
            }
        }
        Ok(p)
    }
}

/// Register contents for one call, plus the buffers its pointers point into.
#[derive(Default)]
pub(crate) struct Packed {
    pub(crate) ints: [usize; MAX_INT_ARGS],
    pub(crate) dbls: [f64; MAX_DOUBLE_ARGS],
    strings: Vec<CString>,
    buffers: Vec<Vec<u8>>,
}

fn to_double(i: usize, v: &Value) -> ValueResult<f64> {
    match v {
        Value::Double(d) => Ok(*d),
        Value::Long(n) => Ok(*n as f64),
        other => Err(arg_type_error(i, "double", other)),
    }
}

impl Packed {
    fn int_slot(&mut self, i: usize, ty: CType, v: &Value) -> ValueResult<usize> {
        let bad = || arg_type_error(i, ty.name(), v);
        match ty {
            CType::Int => match v {
                Value::Long(n) => Ok(*n as i32 as i64 as usize),
                _ => Err(bad()),
            },
            CType::Long => match v {
                Value::Long(n) => Ok(*n as usize),
                _ => Err(bad()),
            },
            CType::Pointer => match v {
                Value::Nil => Ok(0),
                Value::Long(n) => Ok(*n as usize),
                _ => Err(bad()),
            },
            CType::Str => match v {
                Value::Nil => Ok(0),
                Value::Str(s) => {
                    let c = CString::new(s.get().as_bytes()).map_err(|_| bad())?;
                    let addr = c.as_ptr() as usize;
                    self.strings.push(c);
                    Ok(addr)
                }
                _ => Err(bad()),
            },
            CType::Bytes => match v {
                Value::Nil => Ok(0),
                Value::ByteArray(arr) => {
                    let copy: Vec<u8> =
                        arr.get().lock().unwrap().iter().map(|b| *b as u8).collect();
                    let addr = copy.as_ptr() as usize;
                    self.buffers.push(copy);
                    Ok(addr)
                }
                _ => Err(bad()),
            },
            CType::Void | CType::Double => unreachable!("not an integer-class arg type"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cljrs_gc::GcPtr;
    use cljrs_value::{Keyword, PersistentVector};

    fn kws(names: &[&str]) -> Value {
        let mut v = PersistentVector::empty();
        for n in names {
            v = v.conj(Value::keyword(Keyword::parse(n)));
        }
        Value::Vector(GcPtr::new(v))
    }

    fn kw(n: &str) -> Value {
        Value::keyword(Keyword::parse(n))
    }

    #[test]
    fn every_type_parses_and_round_trips_its_name() {
        for n in [
            "void", "int", "long", "double", "pointer", "string", "bytes",
        ] {
            assert_eq!(CType::parse(n).unwrap().name(), n);
        }
        assert!(CType::parse("float").is_none());
    }

    #[test]
    fn doubles_and_ints_fill_separate_files_in_order() {
        let sig = Signature::resolve(
            &kws(&["long", "double", "long", "double", "int"]),
            &kw("long"),
        )
        .unwrap();
        let p = sig
            .pack(&[
                Value::Long(1),
                Value::Double(2.5),
                Value::Long(3),
                Value::Long(4),
                Value::Long(-1),
            ])
            .unwrap();
        assert_eq!(&p.ints[..3], &[1, 3, (-1i64) as usize]);
        assert_eq!(&p.dbls[..2], &[2.5, 4.0]);
        assert_eq!(p.ints[3..], [0, 0, 0]);
    }

    #[test]
    fn int_args_truncate_to_32_bits_sign_extended() {
        let sig = Signature::resolve(&kws(&["int"]), &kw("int")).unwrap();
        let p = sig.pack(&[Value::Long(0x1_0000_0005)]).unwrap();
        assert_eq!(p.ints[0], 5);
    }

    #[test]
    fn limits_are_six_ints_and_eight_doubles() {
        assert!(Signature::resolve(&kws(&["long"; 6]), &kw("void")).is_ok());
        assert!(Signature::resolve(&kws(&["long"; 7]), &kw("void")).is_err());
        assert!(Signature::resolve(&kws(&["double"; 8]), &kw("double")).is_ok());
        assert!(Signature::resolve(&kws(&["double"; 9]), &kw("double")).is_err());
    }

    #[test]
    fn misplaced_and_unknown_types_are_refused() {
        assert!(Signature::resolve(&kws(&["void"]), &kw("int")).is_err());
        assert!(Signature::resolve(&kws(&[]), &kw("bytes")).is_err());
        assert!(Signature::resolve(&kws(&["float"]), &kw("int")).is_err());
        assert!(Signature::resolve(&kws(&[]), &kw("struct")).is_err());
    }

    #[test]
    fn arity_and_value_type_are_checked_at_pack() {
        let sig = Signature::resolve(&kws(&["long", "string"]), &kw("void")).unwrap();
        assert!(sig.pack(&[Value::Long(1)]).is_err());
        assert!(sig.pack(&[Value::Double(1.0), Value::Nil]).is_err());
        assert!(sig.pack(&[Value::Long(1), Value::Long(2)]).is_err());
        let p = sig.pack(&[Value::Long(1), Value::Nil]).unwrap();
        assert_eq!(p.ints[1], 0);
    }

    #[test]
    fn string_arg_points_at_a_nul_terminated_copy() {
        let sig = Signature::resolve(&kws(&["string"]), &kw("void")).unwrap();
        let p = sig.pack(&[Value::string("héllo".to_string())]).unwrap();
        let s = unsafe { std::ffi::CStr::from_ptr(p.ints[0] as *const std::ffi::c_char) };
        assert_eq!(s.to_str().unwrap(), "héllo");
        let nul = sig.pack(&[Value::string("a\0b".to_string())]);
        assert!(nul.is_err());
    }
}
