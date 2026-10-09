use cljrs_gc::GcPtr;
use cljrs_value::error::ExceptionInfo;
use cljrs_value::{Value, ValueError, ValueResult};

use crate::env::env::Env;
use crate::env::error::{EvalResult, value_error_to_eval_error};

/// `(new T args...)`, intercepted so it can read the environment.
///
/// A deftype/defrecord evaluates to its qualified type symbol (`my.ns.T`), and
/// its macro defs the `T.` constructor beside `->T`. `new` resolves the type
/// to that constructor var (in the type's own namespace, else as written in
/// `*ns*`) and calls it. Anything else (`Exception`) falls back to the
/// built-in host-class table of [`builtin_new`].
pub fn eval_new(args: Vec<Value>, env: &mut Env) -> EvalResult {
    if let Some(Value::Symbol(s)) = args.first() {
        let sym = s.get();
        let full = sym.full_name();
        let ctor_var = match full.rfind('.') {
            Some(i) if sym.namespace.is_none() && i > 0 && i + 1 < full.len() => env
                .globals
                .lookup_var_in_ns(&full[..i], &format!("{}.", &full[i + 1..])),
            _ => None,
        }
        .or_else(|| {
            let ns = crate::interp::apply::resolve_current_ns(env);
            env.globals.lookup_var_in_ns(&ns, &format!("{full}."))
        });
        if let Some(var) = ctor_var
            && let Some(ctor) = crate::env::dynamics::deref_var(&var)
        {
            return crate::env::apply::apply_value(&ctor, args[1..].to_vec(), env);
        }
    }
    builtin_new(&args).map_err(value_error_to_eval_error)
}

// Work-alike for 'new' for a limited set of "classes"
pub fn builtin_new(args: &[Value]) -> ValueResult<Value> {
    if let Value::Symbol(symbol) = &args[0] {
        match symbol.get().full_name().as_str() {
            "Exception" | "java.lang.Exception" => builtin_exception_dot(&args[1..]),
            _ => Err(ValueError::Other(format!(
                "unknown type {}",
                symbol.get().full_name()
            ))),
        }
    } else {
        Err(ValueError::WrongType {
            expected: "symbol",
            got: args[0].type_name().to_string(),
        })
    }
}

pub fn builtin_exception_dot(args: &[Value]) -> ValueResult<Value> {
    if args.is_empty() {
        Ok(Value::Error(GcPtr::new(ExceptionInfo::new(
            ValueError::Other("".to_string()),
            "".to_string(),
            None,
            None,
        ))))
    } else if args.len() == 1 {
        match &args[0] {
            Value::Str(s) => Ok(Value::Error(GcPtr::new(ExceptionInfo::new(
                ValueError::Other(s.get().to_string()),
                s.get().to_string(),
                None,
                None,
            )))),
            Value::Error(e) => Ok(Value::Error(GcPtr::new(ExceptionInfo::new(
                ValueError::Other("".to_string()),
                "".to_string(),
                None,
                Some(e.clone()),
            )))),
            v => Err(ValueError::WrongType {
                expected: "str or error",
                got: v.type_name().to_string(),
            }),
        }
    } else {
        let message = match &args[0] {
            Value::Str(s) => s.get().to_string(),
            v => {
                return Err(ValueError::WrongType {
                    expected: "str",
                    got: v.type_name().to_string(),
                });
            }
        };
        let cause = match &args[1] {
            Value::Error(e) => Some(e.clone()),
            v => {
                return Err(ValueError::WrongType {
                    expected: "error",
                    got: v.type_name().to_string(),
                });
            }
        };
        Ok(Value::Error(GcPtr::new(ExceptionInfo::new(
            ValueError::Other(message.to_string()),
            message.to_string(),
            None,
            cause,
        ))))
    }
}
