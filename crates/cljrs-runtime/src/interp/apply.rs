//! Function application and the recur trampoline.

use crate::builtins::form::form_to_value;
use cljrs_gc::GcPtr;
use cljrs_reader::{Form, FormKind};
use cljrs_value::{
    Atom, CljxFn, CljxFnArity, Delay, LazySeq, MapValue, PersistentList, Symbol, Thunk, Value,
    Volatile,
};
use std::collections::HashMap;
use std::sync::Arc;

use crate::env::env::Env;
use crate::env::error::{EvalError, EvalResult, value_error_to_eval_error};
use crate::interp::destructure::value_to_seq_vec;
use crate::interp::eval::eval;

/// Convert an EvalError to a Value for storage (e.g. agent errors).
/// Preserves Thrown values (ex-info); other errors become strings.
#[allow(dead_code)]
fn eval_error_to_value(e: EvalError) -> Value {
    match e {
        EvalError::Thrown(v) => v,
        other => Value::string(format!("{other}")),
    }
}

// ── Watch notification ───────────────────────────────────────────────────────

/// Fire all watches on a watchable (atom, var, agent).
/// Each watch fn is called as `(f key ref old new)`.
/// Errors thrown by watch fns are re-thrown (matching Clojure behavior).
fn fire_watches(
    watches: &std::sync::Mutex<Vec<(Value, Value)>>,
    reference: &Value,
    old: &Value,
    new: &Value,
    env: &mut Env,
) {
    let ws: Vec<(Value, Value)> = watches.lock().unwrap().clone();
    for (key, f) in &ws {
        let args = vec![key.clone(), reference.clone(), old.clone(), new.clone()];
        if let Err(e) = crate::env::apply::apply_value(f, args, env) {
            // Re-throw watch errors (Clojure behavior: exception propagates to caller)
            // We can't return EvalResult from here, so we store and re-throw below.
            // For now, propagate by re-invoking so it surfaces.
            // Actually, in Clojure, watch exceptions propagate to the mutating call.
            // We need to handle this differently — but for simplicity, just ignore for now
            // and let the caller check. Actually let's just use a thread-local to propagate.
            WATCH_ERROR.with(|cell| {
                cell.borrow_mut().replace(e);
            });
            return;
        }
    }
}

thread_local! {
    static WATCH_ERROR: std::cell::RefCell<Option<EvalError>> = const { std::cell::RefCell::new(None) };
}

/// Check if a watch error occurred and propagate it.
fn check_watch_error() -> EvalResult<()> {
    WATCH_ERROR.with(|cell| {
        if let Some(e) = cell.borrow_mut().take() {
            Err(e)
        } else {
            Ok(())
        }
    })
}

// ── ClosureThunk ──────────────────────────────────────────────────────────────

/// A Thunk that calls a zero-arg Clojure closure when forced.
#[derive(Debug)]
pub struct ClosureThunk {
    pub f: CljxFn,
    pub globals: std::sync::Arc<crate::env::env::GlobalEnv>,
    pub ns: std::sync::Arc<str>,
}

/// A Thunk that wraps any zero-arg callable `Value` (a `Value::Fn`, a
/// `Value::NativeFunction`, etc.) and forces by routing through
/// `apply_value`.  Used by [`make_lazy_seq_from_fn`] when the supplied
/// value isn't a plain Clojure fn — for example the IR interpreter's
/// `AllocClosure` produces a `NativeFunction` wrapping the IR closure.
struct CallableValueThunk {
    callee: Value,
    globals: std::sync::Arc<crate::env::env::GlobalEnv>,
    ns: std::sync::Arc<str>,
}

impl std::fmt::Debug for CallableValueThunk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallableValueThunk")
            .field("callee", &self.callee.type_name())
            .field("ns", &self.ns)
            .finish()
    }
}

impl cljrs_gc::Trace for CallableValueThunk {
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        self.callee.trace(visitor);
    }
}

impl Thunk for CallableValueThunk {
    fn force(&self) -> Result<Value, String> {
        let mut env = Env::new(self.globals.clone(), &self.ns);
        crate::env::apply::apply_value(&self.callee, Vec::new(), &mut env)
            .map_err(|e| format!("{e}"))
    }
}

/// Wrap a zero-arg callable value in a `Value::LazySeq` whose `force`
/// calls it.
///
/// Accepts any `Value` whose type-name is "fn" — `Value::Fn`,
/// `Value::NativeFunction`, `Value::BoundFn`, etc. — and unwraps any
/// surrounding `WithMeta`.  The fast path stays in `Value::Fn` (a direct
/// `ClosureThunk`); other callables route through a thunk that dispatches
/// via `apply_value` on force.
///
/// This is the value-level analogue of [`handle_make_lazy_seq`], usable
/// from contexts that already have a `Value` (e.g. the IR interpreter)
/// rather than a `Form`.
pub fn make_lazy_seq_from_fn(
    f_val: &Value,
    globals: std::sync::Arc<crate::env::env::GlobalEnv>,
    ns: std::sync::Arc<str>,
) -> EvalResult {
    let unwrapped = f_val.unwrap_meta();
    if let Value::Fn(g) = unwrapped {
        let thunk = ClosureThunk {
            f: g.get().clone(),
            globals,
            ns,
        };
        return Ok(Value::LazySeq(GcPtr::new(LazySeq::new(Box::new(thunk)))));
    }
    // Anything else with type-name "fn" is acceptable; route through
    // apply_value at force-time.  Reject non-callable values up front.
    if unwrapped.type_name() != "fn" {
        return Err(EvalError::Runtime(format!(
            "make-lazy-seq requires a fn, got {}",
            unwrapped.type_name(),
        )));
    }
    let thunk = CallableValueThunk {
        callee: unwrapped.clone(),
        globals,
        ns,
    };
    Ok(Value::LazySeq(GcPtr::new(LazySeq::new(Box::new(thunk)))))
}

impl cljrs_gc::Trace for ClosureThunk {
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        self.f.trace(visitor);
    }
}

impl Thunk for ClosureThunk {
    fn force(&self) -> Result<Value, String> {
        // Root the closed-over values so they survive GC.  The thunk may live
        // on the Rust stack outside any Env frame (e.g., after LazySeq::realize
        // drops its Mutex guard), so GC wouldn't trace them otherwise.
        let _closed_root = crate::env::gc_roots::root_values(&self.f.closed_over_vals);
        let mut env = Env::with_closure(self.globals.clone(), &self.ns, &self.f);
        call_cljrs_fn(&self.f, &[], &mut env).map_err(|e| format!("{e}"))
    }
}

/// A native fn implemented on already-evaluated arguments.
///
/// `volatile!` and `agent` do not read the environment; they are adapted to
/// this shape at the table rather than given a second signature.
pub type InterceptedFn = fn(Vec<Value>, &mut Env) -> EvalResult;

/// THE definition of which natives the evaluators intercept, and of what each
/// one does.
///
/// These need the *environment*, not just their argument values, so their
/// entries in the builtin table are sentinel stubs that error when invoked
/// directly. Every consumer is a projection of this one function:
/// [`is_form_intercepted`] asks whether it answers, [`dispatch_intercepted`]
/// calls what it answers with. There is no second list to keep in step, which
/// is the whole point — the IR interpreter used to carry one, twenty names
/// short, and `(resolve 'map)` inside a function threw once the function got
/// hot.
pub fn intercepted_native(name: &str) -> Option<InterceptedFn> {
    Some(match name {
        "apply" => eval_apply,
        "atom" => eval_atom,
        "reset!" => eval_reset_bang,
        "swap!" => eval_swap_bang,
        "volatile!" => |args, _env| eval_volatile(args),
        "vreset!" => |args, _env| eval_vreset_bang(args),
        "agent" => |args, _env| eval_agent(args),
        "make-lazy-seq" => eval_make_lazy_seq,
        "make-delay" => eval_make_delay,
        "vswap!" => eval_vswap_bang,
        "send" | "send-off" => eval_send_to_agent,
        "with-bindings*" => eval_with_bindings_star,
        "alter-var-root" => eval_alter_var_root,
        "vary-meta" => eval_vary_meta,
        "eval" => eval_eval,
        "find-ns" | "the-ns" => eval_find_ns,
        "ns-interns" | "ns-publics" => eval_ns_interns,
        "ns-refers" => eval_ns_refers,
        "ns-map" => eval_ns_map,
        "all-ns" => eval_all_ns,
        "create-ns" => eval_create_ns,
        "ns-aliases" => eval_ns_aliases,
        "remove-ns" => eval_remove_ns,
        "alter-meta!" => eval_alter_meta,
        "ns-resolve" => eval_ns_resolve,
        "resolve" => eval_resolve,
        "intern" => eval_intern,
        "bound-fn*" => eval_bound_fn_star,
        _ => return None,
    })
}

/// Whether [`eval_call`] intercepts this native rather than calling it.
///
/// A projection of [`intercepted_native`]: it cannot disagree with what
/// actually gets dispatched. The async tree-walker and the IR interpreter both
/// read this to decide when a call belongs on the synchronous path.
pub fn is_form_intercepted(name: &str) -> bool {
    intercepted_native(name).is_some()
}

/// Run an intercepted native on already-evaluated arguments.
///
/// `None` means the name is not intercepted — the same answer
/// [`is_form_intercepted`] gives, from the same place.
pub fn dispatch_intercepted(name: &str, args: Vec<Value>, env: &mut Env) -> Option<EvalResult> {
    intercepted_native(name).map(|f| f(args, env))
}

/// Names whose *argument evaluation* must allocate in the static arena under
/// the `no-gc` feature, because the container being built outlives every
/// scratch region.
#[cfg(feature = "no-gc")]
const STATIC_ARENA_ARGS: &[&str] = &[
    "atom",
    "volatile!",
    "reset!",
    "vreset!",
    "vswap!",
    "swap!",
    "alter-var-root",
    "intern",
];

/// Evaluate argument forms left to right, rooting the partial results so an
/// earlier value survives a GC triggered by a later `eval`.
fn eval_args(name: &str, arg_forms: &[Form], env: &mut Env) -> EvalResult<Vec<Value>> {
    #[cfg(feature = "no-gc")]
    let _static_ctx = STATIC_ARENA_ARGS
        .contains(&name)
        .then(cljrs_gc::alloc_ctx::StaticCtxGuard::new);
    #[cfg(not(feature = "no-gc"))]
    let _ = name;

    let mut args: Vec<Value> = Vec::with_capacity(arg_forms.len());
    for f in arg_forms {
        let _root = crate::env::gc_roots::root_values(&args);
        args.push(eval(f, env)?);
    }
    Ok(args)
}

/// Evaluate a call expression `(func-form arg1 arg2 ...)`.
///
/// Handles:
/// - Macro expansion (if callee is a macro).
/// - The `apply` function (spread last arg).
/// - The `swap!` function (needs env to call the function).
/// - Regular function calls.
pub fn eval_call(func_form: &Form, arg_forms: &[Form], env: &mut Env) -> EvalResult {
    // Interop: (.methodName target args...) — method call syntax.
    if let FormKind::Symbol(s) = &func_form.kind
        && is_method_sugar(s)
    {
        return eval_method_call(&s[1..], arg_forms, env);
    }

    // Evaluate the callee first.
    let callee = eval(func_form, env)?;

    // Root the callee so it survives any GC triggered during argument evaluation.
    let _callee_root = crate::env::gc_roots::root_value(&callee);

    // Macro check: expand then re-eval.
    if let Value::Macro(mfn) = &callee {
        let expanded = macro_apply(mfn.get(), func_form, arg_forms, env)?;
        return eval(&expanded, env);
    }

    // Intercepted natives (`apply`, `swap!`, the ns-* family, …) need the
    // environment, so they are dispatched here on evaluated arguments rather
    // than through the builtin table, whose entries for them are sentinels.
    if let Value::NativeFunction(nf) = &callee {
        let name = nf.get().name.clone();
        crate::env::policy::check_native(&name)?;
        if is_form_intercepted(&name) {
            let args = eval_args(&name, arg_forms, env)?;
            let _args_root = crate::env::gc_roots::root_values(&args);
            return dispatch_intercepted(&name, args, env).unwrap_or_else(|| {
                Err(EvalError::Runtime(format!(
                    "internal: {name} is intercepted but has no dispatch arm"
                )))
            });
        }
    }

    // Evaluate arguments one-at-a-time, rooting partial results so that
    // previously-evaluated args survive any GC triggered during later evals.
    let mut args: Vec<Value> = Vec::with_capacity(arg_forms.len());
    for f in arg_forms {
        // Root the already-evaluated args before each eval that could trigger GC.
        let _args_root = crate::env::gc_roots::root_values(&args);
        args.push(eval(f, env)?);
    }

    // For Clojure functions, dispatch through `GlobalEnv::call_cljrs_fn` so
    // the runtime's execution mode picks the path: the IR-aware dispatcher in
    // `crate::tiered::apply` for a tiered runtime, this module's plain tree
    // walker otherwise.
    if let Value::Fn(f) = &callee {
        // `^:async` functions dispatch through the async runtime (when one is
        // registered), spawning the body and returning a Future immediately.
        if let Some(fut) = crate::env::apply::dispatch_if_async(&callee, &args, env) {
            return Ok(fut);
        }
        let _args_root = crate::env::gc_roots::root_values(&args);
        crate::env::gc_roots::gc_safepoint(env);
        return env.call_cljrs_fn(f.get(), &args);
    }

    crate::env::apply::apply_value(&callee, args, env)
}

// ── Interop method calls ─────────────────────────────────────────────────────

/// Evaluate `(.methodName target args...)` interop syntax.
///
/// Currently supports a small set of methods on built-in types:
/// - `.indexOf` on strings and vectors
/// - `.startsWith`, `.endsWith`, `.contains`, `.substring`, `.length`,
///   `.charAt`, `.toUpperCase`, `.toLowerCase`, `.trim`, `.replace`,
///   `.split` on strings
fn eval_method_call(method: &str, arg_forms: &[Form], env: &mut Env) -> EvalResult {
    if arg_forms.is_empty() {
        return Err(EvalError::Runtime(format!(
            ".{method} requires a target object"
        )));
    }
    let target = eval(&arg_forms[0], env)?;
    let args: Vec<Value> = arg_forms[1..]
        .iter()
        .map(|f| eval(f, env))
        .collect::<EvalResult<_>>()?;

    dispatch_method(method, &target, &args)
}

/// The `.method` / `.-field` head predicate.
///
/// Re-exported from `cljrs_ir::lower` so the evaluator, the async evaluator,
/// the ANF lowerer and the AOT driver all read one definition.
pub use cljrs_ir::lower::is_method_sugar;

/// Dispatch `(.method target args…)` on an already-evaluated target.
///
/// Form-free, so the Tier-1 IR interpreter can route dot-marked
/// `CallDirect` instructions here (see `dispatch_sentinel_by_name` in
/// `crate::tiered`) and behave exactly like the tree-walker's interop path.
pub fn dispatch_method(method: &str, target: &Value, args: &[Value]) -> EvalResult {
    match target {
        Value::Str(s) => dispatch_string_method(method, s.get(), args),
        Value::Vector(v) => dispatch_vector_method(method, v, args),
        Value::List(_) | Value::Cons(_) | Value::LazySeq(_) => {
            dispatch_seq_method(method, target, args)
        }
        Value::TypeInstance(ti) => {
            // `.-field` reads a deftype/defrecord field. There are no host
            // methods to call on an interpreter instance, so a plain `.method`
            // is unsupported (protocol methods are called as `(proto-fn inst)`).
            if let Some(field) = method.strip_prefix('-') {
                let key = Value::keyword(cljrs_value::Keyword::simple(field));
                let inst = ti.get();
                // A mutable field lives in the interior-mutable cell; an
                // immutable one in the field map.
                if let Some(atom) = &inst.mutable
                    && let Value::Map(m) = atom.get().deref()
                    && let Some(v) = m.get(&key)
                {
                    return Ok(v);
                }
                Ok(inst.fields.get(&key).unwrap_or(Value::Nil))
            } else {
                Err(EvalError::Runtime(format!(
                    ".{method} not supported on {} (only .-field access is)",
                    target.type_name()
                )))
            }
        }
        _ => Err(EvalError::Runtime(format!(
            ".{method} not supported on type {}",
            target.type_name()
        ))),
    }
}

fn dispatch_string_method(method: &str, s: &str, args: &[Value]) -> EvalResult {
    match method {
        "indexOf" => {
            let needle = match args.first() {
                Some(Value::Str(s)) => s.get().to_string(),
                Some(Value::Char(c)) => c.to_string(),
                Some(v) => {
                    return Err(EvalError::Runtime(format!(
                        ".indexOf expects string or char argument, got {}",
                        v.type_name()
                    )));
                }
                None => return Err(EvalError::Runtime(".indexOf requires an argument".into())),
            };
            match s.find(&needle) {
                Some(pos) => Ok(Value::Long(pos as i64)),
                None => Ok(Value::Long(-1)),
            }
        }
        "lastIndexOf" => {
            let needle = match args.first() {
                Some(Value::Str(s)) => s.get().to_string(),
                Some(Value::Char(c)) => c.to_string(),
                _ => {
                    return Err(EvalError::Runtime(
                        ".lastIndexOf requires a string or char argument".into(),
                    ));
                }
            };
            match s.rfind(&needle) {
                Some(pos) => Ok(Value::Long(pos as i64)),
                None => Ok(Value::Long(-1)),
            }
        }
        "startsWith" => {
            let prefix = require_str_arg(args, ".startsWith")?;
            Ok(Value::Bool(s.starts_with(&prefix)))
        }
        "endsWith" => {
            let suffix = require_str_arg(args, ".endsWith")?;
            Ok(Value::Bool(s.ends_with(&suffix)))
        }
        "contains" => {
            let sub = require_str_arg(args, ".contains")?;
            Ok(Value::Bool(s.contains(&sub)))
        }
        "length" => Ok(Value::Long(s.len() as i64)),
        "isEmpty" => Ok(Value::Bool(s.is_empty())),
        "charAt" => {
            let idx = require_long_arg(args, ".charAt")? as usize;
            s.chars()
                .nth(idx)
                .map(Value::Char)
                .ok_or_else(|| EvalError::Runtime(format!(".charAt index {idx} out of bounds")))
        }
        "substring" => {
            let start = require_long_arg(args, ".substring")? as usize;
            let end = args
                .get(1)
                .map(|v| match v {
                    Value::Long(n) => Ok(*n as usize),
                    _ => Err(EvalError::Runtime(
                        ".substring end must be an integer".into(),
                    )),
                })
                .transpose()?;
            let result = match end {
                Some(e) => &s[start..e.min(s.len())],
                None => &s[start..],
            };
            Ok(Value::Str(GcPtr::new(result.to_string())))
        }
        "toUpperCase" => Ok(Value::Str(GcPtr::new(s.to_uppercase()))),
        "toLowerCase" => Ok(Value::Str(GcPtr::new(s.to_lowercase()))),
        "trim" => Ok(Value::Str(GcPtr::new(s.trim().to_string()))),
        "replace" => {
            let from = require_str_arg(args, ".replace")?;
            let to = match args.get(1) {
                Some(Value::Str(s)) => s.get().to_string(),
                Some(Value::Char(c)) => c.to_string(),
                _ => {
                    return Err(EvalError::Runtime(
                        ".replace requires two string arguments".into(),
                    ));
                }
            };
            Ok(Value::Str(GcPtr::new(s.replace(&from, &to))))
        }
        "split" => {
            let sep = require_str_arg(args, ".split")?;
            let parts: Vec<Value> = s
                .split(&sep)
                .map(|p| Value::Str(GcPtr::new(p.to_string())))
                .collect();
            Ok(Value::Vector(GcPtr::new(
                cljrs_value::PersistentVector::from_iter(parts),
            )))
        }
        _ => Err(EvalError::Runtime(format!(
            ".{method} not supported on String"
        ))),
    }
}

fn dispatch_vector_method(
    method: &str,
    v: &GcPtr<cljrs_value::PersistentVector>,
    args: &[Value],
) -> EvalResult {
    match method {
        "indexOf" => {
            let needle = args
                .first()
                .ok_or_else(|| EvalError::Runtime(".indexOf requires an argument".into()))?;
            for (i, item) in v.get().iter().enumerate() {
                if item == needle {
                    return Ok(Value::Long(i as i64));
                }
            }
            Ok(Value::Long(-1))
        }
        "size" | "count" => Ok(Value::Long(v.get().count() as i64)),
        _ => Err(EvalError::Runtime(format!(
            ".{method} not supported on Vector"
        ))),
    }
}

fn dispatch_seq_method(method: &str, target: &Value, args: &[Value]) -> EvalResult {
    match method {
        "indexOf" => {
            let needle = args
                .first()
                .ok_or_else(|| EvalError::Runtime(".indexOf requires an argument".into()))?;
            let items = crate::interp::destructure::value_to_seq_vec(target);
            for (i, item) in items.iter().enumerate() {
                if item == needle {
                    return Ok(Value::Long(i as i64));
                }
            }
            Ok(Value::Long(-1))
        }
        _ => Err(EvalError::Runtime(format!(
            ".{method} not supported on {}",
            target.type_name()
        ))),
    }
}

fn require_str_arg(args: &[Value], method: &str) -> Result<String, EvalError> {
    match args.first() {
        Some(Value::Str(s)) => Ok(s.get().to_string()),
        Some(Value::Char(c)) => Ok(c.to_string()),
        _ => Err(EvalError::Runtime(format!(
            "{method} requires a string argument"
        ))),
    }
}

fn require_long_arg(args: &[Value], method: &str) -> Result<i64, EvalError> {
    match args.first() {
        Some(Value::Long(n)) => Ok(*n),
        _ => Err(EvalError::Runtime(format!(
            "{method} requires an integer argument"
        ))),
    }
}

/// Resolve a type symbol from `extend-type` to a canonical tag.
/// Canonical tags ARE the short names, so this just passes through.
pub fn resolve_type_tag(sym: &str) -> Arc<str> {
    Arc::from(sym)
}

/// Tree-walking execution path (original implementation).
pub fn call_cljrs_fn(f: &CljxFn, args: &[Value], caller_env: &mut Env) -> EvalResult {
    let arity = select_arity(f, args.len())?;

    // Register the caller's env as a GC root so its local bindings survive
    // any collection triggered while we're executing the callee's body.
    let _caller_root = crate::env::gc_roots::push_env_root(caller_env);

    // Create a fresh env with closure bindings, executing in the defining namespace.
    // This ensures macros qualify symbols relative to their definition site.
    let mut env = Env::with_closure(caller_env.globals.clone(), &f.defining_ns, f);

    let mut current_args = Vec::from(args);
    loop {
        // Root current_args on the shadow stack so they survive GC.
        // They haven't been bound into the env yet.
        let _args_root = crate::env::gc_roots::root_values(&current_args);

        // GC safepoint before entering function body
        crate::env::gc_roots::gc_safepoint(&env);

        env.push_frame();

        // Under GC: scope this call's heap allocations in a fresh alloc frame.
        // Everything the body (and parameter binding) allocates is rooted only
        // until the frame drops at the end of this trampoline iteration, so a
        // deep call's locals and a `recur`'s dead intermediates become
        // collectable instead of being pinned for the lifetime of the enclosing
        // top-level form.  `result` is moved out before the frame drops and is
        // re-rooted at the top of the next iteration (`root_values`) or by the
        // caller during return unwinding — no GC safepoint runs in the
        // interval, exactly as the IR/JIT dispatch seam relies on (below).
        #[cfg(not(feature = "no-gc"))]
        let _call_frame = cljrs_gc::push_alloc_frame();

        // Self-reference for named functions: use self_ptr when available so
        // the binding is pointer-equal to the outer Value::Fn holding this fn.
        //
        // BEFORE the params, not after: both bind into this one frame, so
        // binding the fn's own name last OVERWROTE a parameter that shared it.
        // `(defn text [text] {:text text})` then returned the function as its
        // own :text. In Clojure the name is visible in the body but a parameter
        // shadows it, which is exactly what this order gives.
        if let Some(ref name) = f.name {
            let self_val = if let Some(ref p) = f.self_ptr {
                Value::Fn(p.clone())
            } else {
                Value::Fn(GcPtr::new(f.clone()))
            };
            env.bind(name.clone(), self_val);
        }

        // Bind params.
        bind_fn_params(arity, &current_args, &mut env)?;

        // Eval body, catching Recur.
        // Under no-gc: push a scratch region; evaluate all-but-last in it,
        // then pop scratch before the tail expression so the return value
        // lands in the caller's allocation context.
        #[cfg(not(feature = "no-gc"))]
        let result = eval_body_recur_fn(&arity.body, &mut env);
        #[cfg(feature = "no-gc")]
        let result = {
            let mut scratch = cljrs_gc::alloc_ctx::ScratchGuard::new();
            // scratch drops here: resets the region (frees intermediates)
            eval_body_with_scratch(&arity.body, &mut scratch, &mut env)
        };
        env.pop_frame();
        // _call_frame drops at the end of this iteration (after the match
        // below), freeing this call's intermediates.

        match result {
            Ok(v) => return Ok(v),
            Err(EvalError::Recur(new_args)) => {
                // For variadic arities, recur provides n+1 values where the
                // last value IS the rest collection (not spread args to be
                // re-collected). Flatten it so bind_fn_params sees the right
                // number of individual args.
                if arity.rest_param.is_some() {
                    let n = arity.params.len();
                    if new_args.len() == n + 1 {
                        let mut flat = new_args[..n].to_vec();
                        // Spread the rest collection back into individual args.
                        let rest_val = &new_args[n];
                        match rest_val {
                            Value::Nil => {} // no extra args
                            _ => {
                                let rest_items = value_to_seq_vec(rest_val);
                                flat.extend(rest_items);
                            }
                        }
                        current_args = flat;
                    } else {
                        current_args = new_args;
                    }
                } else {
                    current_args = new_args;
                }
            }
            Err(e) => return Err(e),
        }
    }
}

/// Bind function parameters in the current (top) frame, expanding any
/// destructuring patterns the arity carries.
pub fn bind_fn_params(arity: &CljxFnArity, args: &[Value], env: &mut Env) -> EvalResult<()> {
    bind_fn_params_impl(arity, args, env, true)
}

/// Bind only the *named* parameters — the positional slots and the rest list —
/// leaving the arity's destructuring patterns unexpanded.
///
/// For the IR tier: the lowered prologue expands those same patterns into
/// explicit IR bindings (`lower_fn_body_destructured`), and the ANF lowerer
/// never emits `LoadLocal`, so a destructured name is only ever read as an IR
/// register — the env copy is unobservable.  Producing it anyway would not be
/// free, though: an `:or` default is evaluated eagerly, exactly as
/// `(get m :k default)` evaluates its third argument, so a side-effecting
/// default would fire once here and once in the prologue — twice per call.
pub fn bind_fn_params_positional(
    arity: &CljxFnArity,
    args: &[Value],
    env: &mut Env,
) -> EvalResult<()> {
    bind_fn_params_impl(arity, args, env, false)
}

fn bind_fn_params_impl(
    arity: &CljxFnArity,
    args: &[Value],
    env: &mut Env,
    destructure: bool,
) -> EvalResult<()> {
    let n = arity.params.len();
    // Bind positional params.
    for (i, name) in arity.params.iter().enumerate() {
        let val = args.get(i).cloned().unwrap_or(Value::Nil);
        env.bind(name.clone(), val);
    }
    // Bind rest param.
    if let Some(ref rest) = arity.rest_param {
        let rest_items = args[n..].to_vec();
        let rest_val = if rest_items.is_empty() {
            Value::Nil
        } else {
            Value::List(GcPtr::new(PersistentList::from_iter(rest_items)))
        };
        env.bind(rest.clone(), rest_val.clone());
        // Apply rest destructuring if present.
        if destructure && let Some(ref pattern) = arity.destructure_rest {
            // When the rest pattern is a map destructure (e.g. `& {:keys [bar]}`),
            // convert the rest args list into a map of alternating key-value pairs,
            // matching Clojure's keyword-arguments convention.
            let destructure_val = if pattern.is_kwargs_rest_pattern() {
                let items = value_to_seq_vec(&rest_val);
                Value::from_kwargs_rest(items).map_err(value_error_to_eval_error)?
            } else {
                rest_val
            };
            crate::interp::destructure::bind_pattern(pattern, destructure_val, env)?;
        }
    }
    // Apply positional destructuring patterns.
    if destructure {
        for (idx, pattern) in &arity.destructure_params {
            let val = args.get(*idx).cloned().unwrap_or(Value::Nil);
            crate::interp::destructure::bind_pattern(pattern, val, env)?;
        }
    }
    Ok(())
}

/// Eval a function body, propagating Recur up (does not catch it).
#[cfg(not(feature = "no-gc"))]
fn eval_body_recur_fn(body: &[cljrs_reader::Form], env: &mut Env) -> EvalResult {
    let mut result = Value::Nil;
    for form in body {
        result = eval(form, env)?;
    }
    Ok(result)
}

/// Under `no-gc`: evaluate body forms with the scratch region active for all
/// non-tail forms, then pop the scratch before the tail expression so the
/// return value (or `recur` args) are allocated in the caller's context.
#[cfg(feature = "no-gc")]
fn eval_body_with_scratch(
    body: &[cljrs_reader::Form],
    scratch: &mut cljrs_gc::alloc_ctx::ScratchGuard,
    env: &mut Env,
) -> EvalResult {
    if body.is_empty() {
        scratch.pop_for_return();
        return Ok(Value::Nil);
    }
    // Eval all non-tail forms in the scratch region.
    for form in &body[..body.len() - 1] {
        eval(form, env)?;
    }
    // Pop scratch so the tail expression allocates in the caller's context.
    scratch.pop_for_return();
    eval(&body[body.len() - 1], env)
}

/// Select the matching arity for the given argument count.
pub fn select_arity(f: &CljxFn, argc: usize) -> EvalResult<&CljxFnArity> {
    let name = f.name.as_deref().unwrap_or("fn");
    // Try fixed arities first.
    for arity in &f.arities {
        if arity.rest_param.is_none() && arity.params.len() == argc {
            return Ok(arity);
        }
    }
    // Try variadic arities.
    for arity in &f.arities {
        if arity.rest_param.is_some() && argc >= arity.params.len() {
            return Ok(arity);
        }
    }
    // Build expected string.  A macro's params carry the two implicit leading
    // arguments `&form` and `&env` (see `macro_apply`); the caller never wrote
    // them, so both the expected arities and the count reported back are the
    // ones a reader of the source can see.
    let implicit = if f.is_macro { IMPLICIT_MACRO_ARGS } else { 0 };
    let expected: Vec<String> = f
        .arities
        .iter()
        .map(|a| {
            let fixed = a.params.len().saturating_sub(implicit);
            if a.rest_param.is_some() {
                format!("{fixed}+")
            } else {
                fixed.to_string()
            }
        })
        .collect();
    Err(EvalError::Arity {
        name: name.to_string(),
        expected: expected.join(" or "),
        got: argc.saturating_sub(implicit),
    })
}

/// `&form` and `&env`, prepended to every macro call by `macro_apply`.
const IMPLICIT_MACRO_ARGS: usize = 2;

/// Expand a macro: convert unevaluated arg forms to values, call the macro fn,
/// then convert the resulting Value back to a Form.
///
/// Clojure macros receive two implicit leading arguments:
/// - `&form`: the entire call expression as a quoted value
/// - `&env`: a map of local bindings at the call site (symbol → value)
fn macro_apply(
    mfn: &CljxFn,
    func_form: &Form,
    arg_forms: &[Form],
    env: &mut Env,
) -> EvalResult<Form> {
    // Resolve ::kw forms using the caller's namespace before the macro sees them.
    // In Clojure, ::kw is resolved at read time; we approximate that here so a
    // macro splicing its arguments into a new form cannot re-resolve them against
    // the macro's own namespace.
    let resolved_args: Vec<Form> = arg_forms
        .iter()
        .map(|f| crate::builtins::form::resolve_auto_forms(f, env))
        .collect::<EvalResult<Vec<Form>>>()?;

    // The unevaluated arguments as values, converted once: they are both the
    // tail of `&form` and the macro's own arguments.
    let arg_vals: Vec<Value> = resolved_args
        .iter()
        .map(form_to_value)
        .collect::<EvalResult<Vec<Value>>>()?;

    // &form: the whole call expression as a list value.
    let form_val = {
        let mut items = vec![form_to_value(func_form)?];
        items.extend(arg_vals.iter().cloned());
        Value::List(GcPtr::new(PersistentList::from_iter(items)))
    };

    // &env: local variable bindings at call site as a map (symbol → value).
    // Built only for a macro whose body mentions `&env`: the map costs about
    // 5us per local in scope on every expansion, and the tree-walker expands
    // a macro on every use, so `when` in a loop body inside a wide `let` paid
    // it per iteration for a value nothing read.
    let env_val = if mfn.macro_uses_env {
        let (names, vals) = env.all_local_bindings();
        let mut m = MapValue::empty();
        for (name, val) in names.iter().zip(vals.iter()) {
            m = m.assoc(Value::symbol(Symbol::simple(name.as_ref())), val.clone());
        }
        Value::Map(m)
    } else {
        Value::Map(MapValue::empty())
    };

    // Prepend &form and &env, then pass remaining arg forms as unevaluated values.
    let mut args = vec![form_val, env_val];
    args.extend(arg_vals);

    let expanded_val = call_cljrs_fn(mfn, args.as_ref(), env)?;
    let dummy_span = cljrs_types::span::Span::new(Arc::new("<macro>".to_string()), 0, 0, 1, 1);
    crate::interp::macros::value_to_form(&expanded_val, dummy_span)
}

// ── volatile! ────────────────────────────────────────────────────────────────

// ── vreset! ──────────────────────────────────────────────────────────────────

// ── agent ────────────────────────────────────────────────────────────────────

// ── atom ──────────────────────────────────────────────────────────────────────

// ── shared-atom (Phase B3, two-tier ADR) ──────────────────────────────────────
//
// `shared-atom` is the cross-isolate tier of the two-tier atom design: its
// contents live in `SharedValue` (Send + Sync, refcounted) behind a lock-free
// `ArcSwap`, so the same atom can be observed and mutated from any isolate.
// `deref`/`reset!`/`swap!`/`compare-and-set!` all route through these helpers
// when handed a `Value::SharedAtom`, so the surface mirrors a local `atom`
// except that values are promoted on write and demoted on read.

/// `reset!` on a shared-atom: promote the new value and store it atomically.
/// Returns the (isolate-local) value that was written.
fn shared_atom_reset(sa: &Arc<cljrs_value::SharedAtom>, new_val: Value) -> EvalResult {
    let promoted = cljrs_value::promote(&new_val).map_err(|e| EvalError::Runtime(e.to_string()))?;
    sa.reset(promoted);
    Ok(new_val)
}

/// `swap!` on a shared-atom: CAS-retry loop.  Loads the current value, demotes
/// it into an isolate-local `Value`, applies `f` (plus any extra args), promotes
/// the result, and commits with a single compare-and-set — retrying from the
/// fresh value if another isolate raced us in between.
fn shared_atom_swap(
    sa: &Arc<cljrs_value::SharedAtom>,
    f: &Value,
    extra: Vec<Value>,
    env: &mut Env,
) -> EvalResult {
    loop {
        let cur = sa.deref_val();
        let old_val = cljrs_value::demote(&cur);
        let mut call_args = Vec::with_capacity(1 + extra.len());
        call_args.push(old_val);
        call_args.extend(extra.iter().cloned());
        let new_val = crate::env::apply::apply_value(f, call_args, env)?;
        let promoted =
            cljrs_value::promote(&new_val).map_err(|e| EvalError::Runtime(e.to_string()))?;
        if sa.compare_and_set(&cur, promoted) {
            return Ok(new_val);
        }
        // Lost the race; another writer committed first. Re-read and retry.
    }
}

// ── reset! ────────────────────────────────────────────────────────────────────

/// Call the atom's validator (if any) on `new_val`. Throws if invalid.
fn validate_atom_value(atom: &GcPtr<Atom>, new_val: &Value, env: &mut Env) -> EvalResult<()> {
    if let Some(vf) = atom.get().get_validator() {
        let result = crate::env::apply::apply_value(&vf, vec![new_val.clone()], env)?;
        if result == Value::Nil || result == Value::Bool(false) {
            return Err(EvalError::Thrown(Value::string(
                "Invalid value for atom".to_string(),
            )));
        }
    }
    Ok(())
}

// ── swap! ─────────────────────────────────────────────────────────────────────

// ── with-bindings* ────────────────────────────────────────────────────────────

// ── alter-var-root ────────────────────────────────────────────────────────────

// ── vary-meta ────────────────────────────────────────────────────────────────

// ── eval ─────────────────────────────────────────────────────────────────────

/// Execute `eval` with an already-evaluated arg: `[form-value]`.
///
/// The form is evaluated in a fresh top-level environment of the current
/// namespace, so it sees vars but not the caller's locals — as on the JVM.
pub fn eval_eval(args: Vec<Value>, env: &mut Env) -> EvalResult {
    let [value] = args.as_slice() else {
        return Err(EvalError::Arity {
            name: "eval".into(),
            expected: "1".into(),
            got: args.len(),
        });
    };
    let span = cljrs_types::span::Span::new(Arc::new("<eval>".to_string()), 0, 0, 1, 1);
    let form = crate::interp::macros::value_to_form(value, span)?;
    let mut top = Env::new(env.globals.clone(), &env.current_ns);
    eval(&form, &mut top)
}

// ── Value-level special form dispatch (used by IR interpreter) ───────────────
//
// These mirror the `handle_*` functions above but accept already-evaluated
// `Vec<Value>` instead of `&[Form]`.  The IR interpreter calls these directly
// to bypass the sentinel stubs registered in clojure.core.

/// Execute `reset!` with already-evaluated args: `[atom, new-val]`.
pub fn eval_reset_bang(args: Vec<Value>, env: &mut Env) -> EvalResult {
    if args.len() < 2 {
        return Err(EvalError::Arity {
            name: "reset!".into(),
            expected: "2".into(),
            got: args.len(),
        });
    }
    let atom_val = args[0].clone();
    let new_val = args[1].clone();
    let atom = match &atom_val {
        Value::Atom(a) => a.clone(),
        Value::SharedAtom(sa) => return shared_atom_reset(sa, new_val),
        v => {
            return Err(EvalError::Runtime(format!(
                "reset! requires an atom, got {}",
                v.type_name()
            )));
        }
    };
    #[cfg(feature = "no-gc")]
    let _static_ctx = cljrs_gc::alloc_ctx::StaticCtxGuard::new();
    validate_atom_value(&atom, &new_val, env)?;
    let old_val = atom.get().deref();
    atom.get().reset(new_val.clone());
    fire_watches(&atom.get().watches, &atom_val, &old_val, &new_val, env);
    check_watch_error()?;
    Ok(new_val)
}

/// Execute `swap!` with already-evaluated args: `[atom, f, extra...]`.
pub fn eval_swap_bang(mut args: Vec<Value>, env: &mut Env) -> EvalResult {
    if args.len() < 2 {
        return Err(EvalError::Arity {
            name: "swap!".into(),
            expected: "2+".into(),
            got: args.len(),
        });
    }
    let atom_val = args.remove(0);
    let f = args.remove(0);
    let atom = match &atom_val {
        Value::Atom(a) => a.clone(),
        Value::SharedAtom(sa) => return shared_atom_swap(sa, &f, args, env),
        v => {
            return Err(EvalError::Runtime(format!(
                "swap! requires an atom, got {}",
                v.type_name()
            )));
        }
    };
    let old_val = atom.get().deref();
    let mut call_args = vec![old_val.clone()];
    call_args.extend(args);
    #[cfg(feature = "no-gc")]
    let _static_ctx = cljrs_gc::alloc_ctx::StaticCtxGuard::new();
    let new_val = crate::env::apply::apply_value(&f, call_args, env)?;
    validate_atom_value(&atom, &new_val, env)?;
    atom.get().reset(new_val.clone());
    fire_watches(&atom.get().watches, &atom_val, &old_val, &new_val, env);
    check_watch_error()?;
    Ok(new_val)
}

/// Execute `volatile!` with already-evaluated args: `[init-val]`.
pub fn eval_volatile(args: Vec<Value>) -> EvalResult {
    if args.is_empty() {
        return Err(EvalError::Arity {
            name: "volatile!".into(),
            expected: "1".into(),
            got: 0,
        });
    }
    #[cfg(feature = "no-gc")]
    let _static_ctx = cljrs_gc::alloc_ctx::StaticCtxGuard::new();
    Ok(Value::Volatile(GcPtr::new(Volatile::new(
        args.into_iter().next().unwrap(),
    ))))
}

/// Execute `vreset!` with already-evaluated args: `[volatile, new-val]`.
pub fn eval_vreset_bang(args: Vec<Value>) -> EvalResult {
    if args.len() < 2 {
        return Err(EvalError::Arity {
            name: "vreset!".into(),
            expected: "2".into(),
            got: args.len(),
        });
    }
    #[cfg(feature = "no-gc")]
    let _static_ctx = cljrs_gc::alloc_ctx::StaticCtxGuard::new();
    let new_val = args[1].clone();
    match &args[0] {
        Value::Volatile(v) => {
            v.get().reset(new_val.clone());
            Ok(new_val)
        }
        other => Err(EvalError::Runtime(format!(
            "vreset!: expected volatile, got {}",
            other.type_name()
        ))),
    }
}

/// Execute `vswap!` with already-evaluated args: `[volatile, f, extra...]`.
pub fn eval_vswap_bang(mut args: Vec<Value>, env: &mut Env) -> EvalResult {
    if args.len() < 2 {
        return Err(EvalError::Arity {
            name: "vswap!".into(),
            expected: "2+".into(),
            got: args.len(),
        });
    }
    let vol_val = args.remove(0);
    let f = args.remove(0);
    match vol_val {
        Value::Volatile(v) => {
            let cur = v.get().deref();
            let mut call_args = vec![cur];
            call_args.extend(args);
            #[cfg(feature = "no-gc")]
            let _static_ctx = cljrs_gc::alloc_ctx::StaticCtxGuard::new();
            let new_val = crate::env::apply::apply_value(&f, call_args, env)?;
            v.get().reset(new_val.clone());
            Ok(new_val)
        }
        other => Err(EvalError::Runtime(format!(
            "vswap!: expected volatile, got {}",
            other.type_name()
        ))),
    }
}

/// Wrap a zero-arg callable in a `Value::Delay`.
///
/// Analogous to [`make_lazy_seq_from_fn`] but produces a `Delay` instead of
/// a `LazySeq`.
pub fn make_delay_from_fn(
    f_val: &Value,
    globals: std::sync::Arc<crate::env::env::GlobalEnv>,
    ns: std::sync::Arc<str>,
) -> EvalResult {
    let f = match f_val {
        Value::Fn(f) => f.get().clone(),
        other => {
            return Err(EvalError::Runtime(format!(
                "make-delay requires a fn, got {}",
                other.type_name()
            )));
        }
    };
    let thunk = ClosureThunk { f, globals, ns };
    Ok(Value::Delay(GcPtr::new(Delay::new(Box::new(thunk)))))
}

/// Execute `alter-var-root` with already-evaluated args: `[var, f, extra...]`.
pub fn eval_alter_var_root(mut args: Vec<Value>, env: &mut Env) -> EvalResult {
    if args.len() < 2 {
        return Err(EvalError::Arity {
            name: "alter-var-root".into(),
            expected: "2+".into(),
            got: args.len(),
        });
    }
    let var_val = args.remove(0);
    let f = args.remove(0);
    let vp = match &var_val {
        Value::Var(vp) => vp.clone(),
        v => {
            return Err(EvalError::Runtime(format!(
                "alter-var-root: expected var, got {}",
                v.type_name()
            )));
        }
    };
    let old_val = vp.get().deref().unwrap_or(Value::Nil);
    let mut call_args = vec![old_val.clone()];
    call_args.extend(args);
    #[cfg(feature = "no-gc")]
    let _static_ctx = cljrs_gc::alloc_ctx::StaticCtxGuard::new();
    let new_val = crate::env::apply::apply_value(&f, call_args, env)?;
    vp.get().bind(new_val.clone());
    fire_watches(&vp.get().watches, &var_val, &old_val, &new_val, env);
    check_watch_error()?;
    Ok(new_val)
}

/// Execute `vary-meta` with already-evaluated args: `[obj, f, extra...]`.
pub fn eval_vary_meta(mut args: Vec<Value>, env: &mut Env) -> EvalResult {
    if args.len() < 2 {
        return Err(EvalError::Arity {
            name: "vary-meta".into(),
            expected: "2+".into(),
            got: args.len(),
        });
    }
    let obj = args.remove(0);
    let f = args.remove(0);
    let current_meta = match &obj {
        Value::Var(vp) => vp.get().get_meta().unwrap_or(Value::Nil),
        _ => Value::Nil,
    };
    let mut call_args = vec![current_meta];
    call_args.extend(args);
    let new_meta = crate::env::apply::apply_value(&f, call_args, env)?;
    if let Value::Var(vp) = &obj {
        vp.get().set_meta(new_meta);
    }
    Ok(obj)
}

/// Execute `with-bindings*` with already-evaluated args: `[bindings-map, f]`.
pub fn eval_with_bindings_star(args: Vec<Value>, env: &mut Env) -> EvalResult {
    if args.len() < 2 {
        return Err(EvalError::Arity {
            name: "with-bindings*".into(),
            expected: "2".into(),
            got: args.len(),
        });
    }
    let mut frame: HashMap<usize, Value> = HashMap::new();
    if let Value::Map(m) = &args[0] {
        m.for_each(|k, v| {
            if let Value::Var(vp) = k {
                frame.insert(crate::env::dynamics::var_key_of(vp), v.clone());
            }
        });
    } else {
        return Err(EvalError::Runtime(
            "with-bindings*: first arg must be a map".into(),
        ));
    }
    let _guard = crate::env::dynamics::push_frame(frame);
    crate::env::apply::apply_value(&args[1], vec![], env)
}

/// Execute `send` / `send-off` with already-evaluated args: `[agent, f, extra...]`.
pub fn eval_send_to_agent(_args: Vec<Value>, _env: &mut Env) -> EvalResult {
    Err(EvalError::Runtime(
        "send/send-off: agents are not yet implemented".into(),
    ))
}

// ── Namespace reflection (env-needing) ────────────────────────────────────────

fn ns_name_from_val(v: &Value) -> Result<String, EvalError> {
    match v {
        Value::Symbol(s) => Ok(s.get().name.as_ref().to_string()),
        Value::Str(s) => Ok(s.get().clone()),
        Value::Namespace(ns) => Ok(ns.get().name.as_ref().to_string()),
        Value::Keyword(k) => Ok(k.get().name.as_ref().to_string()),
        other => Err(EvalError::Runtime(format!(
            "expected symbol, string, or namespace, got {}",
            other.type_name()
        ))),
    }
}

/// Resolve an already-evaluated arg to a `Namespace`, matching Clojure's
/// `the-ns`: pass a `Namespace` through unchanged, otherwise resolve a
/// symbol/string/keyword name against the global namespace table, throwing
/// if there's no such namespace (rather than a "wrong type" error).
fn the_ns(v: &Value, env: &Env) -> Result<GcPtr<cljrs_value::Namespace>, EvalError> {
    if let Value::Namespace(ns) = v {
        return Ok(ns.clone());
    }
    let name = ns_name_from_val(v)?;
    let map = env.globals.namespaces.read().unwrap();
    match map.get(name.as_str()) {
        Some(ns) => Ok(ns.clone()),
        None => Err(EvalError::Runtime(format!("No namespace: {name} found"))),
    }
}

/// Get the namespace name from `*ns*` (dynamic var), falling back to `env.current_ns`.
/// This is important for `resolve` inside macros, where `env.current_ns` is the
/// macro's defining namespace but `*ns*` is the caller's namespace.
fn resolve_current_ns(env: &Env) -> Arc<str> {
    if let Some(var) = env.globals.lookup_var("clojure.core", "*ns*") {
        let val = crate::env::dynamics::deref_var(&var);
        if let Some(Value::Namespace(ns_ptr)) = val {
            return ns_ptr.get().name.clone();
        }
    }
    env.current_ns.clone()
}

// ── bound-fn* ────────────────────────────────────────────────────────────────

// ── value-level intercepted natives ──────────────────────────────────────────
//
// One implementation per intercepted name, taking already-evaluated arguments.
// `dispatch_intercepted` is the only table that names them, so the tree-walker
// and the tier-1 IR interpreter cannot drift apart.

/// `(apply f & args coll)` — spread the last argument.
pub fn eval_apply(mut args: Vec<Value>, env: &mut Env) -> EvalResult {
    if args.len() < 2 {
        return Err(EvalError::Arity {
            name: "apply".into(),
            expected: "2+".into(),
            got: args.len(),
        });
    }
    let f = args.remove(0);
    let last = args.pop().unwrap();
    // Root f, last and the fixed args during the spread, which may realize a
    // lazy seq and therefore run arbitrary Clojure code.
    let _f_root = crate::env::gc_roots::root_value(&f);
    let _last_root = crate::env::gc_roots::root_value(&last);
    let _args_root = crate::env::gc_roots::root_values(&args);
    args.extend(value_to_seq_vec(&last));
    crate::env::apply::apply_value(&f, args, env)
}

/// `(atom init & {:keys [meta validator]})`.
pub fn eval_atom(args: Vec<Value>, env: &mut Env) -> EvalResult {
    let Some((initial, options)) = args.split_first() else {
        return Err(EvalError::Arity {
            name: "atom".into(),
            expected: "1+".into(),
            got: 0,
        });
    };
    let initial = initial.clone();

    // Parse keyword options; unknown keys / nil keys are ignored.
    let mut meta_opt: Option<Value> = None;
    let mut validator_opt: Option<Value> = None;
    let mut i = 0;
    while i + 1 < options.len() {
        match &options[i] {
            Value::Keyword(k) if k.get().name.as_ref() == "meta" => {
                meta_opt = Some(options[i + 1].clone());
            }
            Value::Keyword(k) if k.get().name.as_ref() == "validator" => {
                let vf = options[i + 1].clone();
                validator_opt = if vf == Value::Nil { None } else { Some(vf) };
            }
            _ => {}
        }
        i += 2;
    }

    // `:meta` must be nil or a map.
    if let Some(ref m) = meta_opt
        && !matches!(m, Value::Nil | Value::Map(_))
    {
        return Err(EvalError::Thrown(Value::string(
            "Atom metadata must be a map or nil".to_string(),
        )));
    }

    // The validator sees the initial value before the atom exists.
    if let Some(ref vf) = validator_opt {
        let result = crate::env::apply::apply_value(vf, vec![initial.clone()], env)?;
        if result == Value::Nil || result == Value::Bool(false) {
            return Err(EvalError::Thrown(Value::string(
                "Invalid initial value for atom".to_string(),
            )));
        }
    }

    // Under no-gc: the container outlives every scratch region.
    #[cfg(feature = "no-gc")]
    let _static_ctx = cljrs_gc::alloc_ctx::StaticCtxGuard::new();
    let atom = GcPtr::new(Atom::new(initial));
    if let Some(m) = meta_opt {
        atom.get()
            .set_meta(if m == Value::Nil { None } else { Some(m) });
    }
    if let Some(vf) = validator_opt {
        atom.get().set_validator(Some(vf));
    }
    Ok(Value::Atom(atom))
}

/// `(agent init & opts)` — not implemented yet.
pub fn eval_agent(_args: Vec<Value>) -> EvalResult {
    Err(EvalError::Runtime("agent is not yet implemented".into()))
}

/// Pull the single zero-arg fn argument out of a `make-delay` / `make-lazy-seq` call.
fn thunk_arg(name: &str, args: Vec<Value>, env: &Env) -> EvalResult<ClosureThunk> {
    let [f_val] = args.as_slice() else {
        return Err(EvalError::Arity {
            name: name.into(),
            expected: "1".into(),
            got: args.len(),
        });
    };
    match f_val {
        Value::Fn(f) => Ok(ClosureThunk {
            f: f.get().clone(),
            globals: env.globals.clone(),
            ns: env.current_ns.clone(),
        }),
        other => Err(EvalError::Runtime(format!(
            "{name} requires a fn, got {}",
            other.type_name()
        ))),
    }
}

/// `(make-lazy-seq f)` — wrap a zero-arg fn in a lazy sequence.
pub fn eval_make_lazy_seq(args: Vec<Value>, env: &mut Env) -> EvalResult {
    let thunk = thunk_arg("make-lazy-seq", args, env)?;
    Ok(Value::LazySeq(GcPtr::new(LazySeq::new(Box::new(thunk)))))
}

/// `(make-delay f)` — wrap a zero-arg fn in a Delay.
pub fn eval_make_delay(args: Vec<Value>, env: &mut Env) -> EvalResult {
    let thunk = thunk_arg("make-delay", args, env)?;
    Ok(Value::Delay(GcPtr::new(Delay::new(Box::new(thunk)))))
}

/// The single namespace argument shared by the `ns-*` family.
fn ns_arg(name: &str, args: &[Value]) -> EvalResult<Value> {
    args.first().cloned().ok_or(EvalError::Arity {
        name: name.into(),
        expected: "1".into(),
        got: 0,
    })
}

/// `(ns-interns ns)` / `(ns-publics ns)` — map of Symbol → Var for interned vars.
pub fn eval_ns_interns(args: Vec<Value>, env: &mut Env) -> EvalResult {
    let ns = the_ns(&ns_arg("ns-interns", &args)?, env)?;
    crate::builtins::builtins::builtin_ns_interns(&[Value::Namespace(ns)])
        .map_err(crate::env::error::value_error_to_eval_error)
}

/// `(ns-refers ns)` — map of Symbol → Var for all referred vars.
pub fn eval_ns_refers(args: Vec<Value>, env: &mut Env) -> EvalResult {
    let ns = the_ns(&ns_arg("ns-refers", &args)?, env)?;
    crate::builtins::builtins::builtin_ns_refers(&[Value::Namespace(ns)])
        .map_err(crate::env::error::value_error_to_eval_error)
}

/// `(ns-map ns)` — map of Symbol → Var for all visible names (interns + refers).
pub fn eval_ns_map(args: Vec<Value>, env: &mut Env) -> EvalResult {
    let ns = the_ns(&ns_arg("ns-map", &args)?, env)?;
    crate::builtins::builtins::builtin_ns_map(&[Value::Namespace(ns)])
        .map_err(crate::env::error::value_error_to_eval_error)
}

/// `(find-ns sym)` / `(the-ns sym)` — look up a namespace by name; nil if absent.
pub fn eval_find_ns(args: Vec<Value>, env: &mut Env) -> EvalResult {
    let name = ns_name_from_val(&ns_arg("find-ns", &args)?)?;
    let map = env.globals.namespaces.read().unwrap();
    match map.get(name.as_str()) {
        Some(ns) => Ok(Value::Namespace(ns.clone())),
        None => Ok(Value::Nil),
    }
}

/// `(all-ns)` — sequence of all live namespaces.
pub fn eval_all_ns(_args: Vec<Value>, env: &mut Env) -> EvalResult {
    let map = env.globals.namespaces.read().unwrap();
    let items: Vec<Value> = map
        .values()
        .map(|ns| Value::Namespace(ns.clone()))
        .collect();
    drop(map);
    Ok(Value::List(cljrs_gc::GcPtr::new(
        cljrs_value::PersistentList::from_iter(items),
    )))
}

/// `(create-ns sym)` — create (or return existing) namespace.
pub fn eval_create_ns(args: Vec<Value>, env: &mut Env) -> EvalResult {
    let name = ns_name_from_val(&ns_arg("create-ns", &args)?)?;
    let ns = env.globals.get_or_create_ns(&name);
    Ok(Value::Namespace(ns))
}

/// `(ns-aliases ns)` — map of Symbol → Namespace for all aliases in ns.
pub fn eval_ns_aliases(args: Vec<Value>, env: &mut Env) -> EvalResult {
    let ns_name = ns_name_from_val(&ns_arg("ns-aliases", &args)?)?;
    let aliases = {
        let map = env.globals.namespaces.read().unwrap();
        match map.get(ns_name.as_str()) {
            Some(ns) => ns.get().aliases.lock().unwrap().clone(),
            None => return Ok(Value::Map(cljrs_value::MapValue::empty())),
        }
    };
    let mut m = cljrs_value::MapValue::empty();
    for (alias, full_ns_name) in &aliases {
        let sym = Value::symbol(cljrs_value::Symbol::simple(alias.clone()));
        let nsmap = env.globals.namespaces.read().unwrap();
        if let Some(target_ns) = nsmap.get(full_ns_name.as_ref()) {
            let target = Value::Namespace(target_ns.clone());
            drop(nsmap);
            m = m.assoc(sym, target);
        }
    }
    Ok(Value::Map(m))
}

/// `(remove-ns sym)` — remove a namespace.
pub fn eval_remove_ns(args: Vec<Value>, env: &mut Env) -> EvalResult {
    let name = ns_name_from_val(&ns_arg("remove-ns", &args)?)?;
    env.globals
        .namespaces
        .write()
        .unwrap()
        .remove(name.as_str());
    Ok(Value::Nil)
}

/// `(alter-meta! ref f & args)` — apply f to ref's meta + args; store and return it.
pub fn eval_alter_meta(mut args: Vec<Value>, env: &mut Env) -> EvalResult {
    if args.len() < 2 {
        return Err(EvalError::Arity {
            name: "alter-meta!".into(),
            expected: "2+".into(),
            got: args.len(),
        });
    }
    let obj = args.remove(0);
    let f = args.remove(0);
    let current_meta = match &obj {
        Value::Var(vp) => vp
            .get()
            .get_meta()
            .unwrap_or(Value::Map(cljrs_value::MapValue::empty())),
        _ => Value::Map(cljrs_value::MapValue::empty()),
    };
    let mut call_args = vec![current_meta];
    call_args.extend(args);
    let new_meta = crate::env::apply::apply_value(&f, call_args, env)?;
    if let Value::Var(vp) = &obj {
        vp.get().set_meta(new_meta.clone());
    }
    Ok(new_meta)
}

/// `(ns-resolve ns sym)` — the Var for sym in ns, or nil.
pub fn eval_ns_resolve(args: Vec<Value>, env: &mut Env) -> EvalResult {
    let [ns_arg, sym_arg, ..] = args.as_slice() else {
        return Err(EvalError::Arity {
            name: "ns-resolve".into(),
            expected: "2".into(),
            got: args.len(),
        });
    };
    let ns_name = ns_name_from_val(ns_arg)?;
    let sym_name = match sym_arg {
        Value::Symbol(s) => s.get().name.as_ref().to_string(),
        Value::Str(s) => s.get().clone(),
        other => {
            return Err(EvalError::Runtime(format!(
                "ns-resolve: second arg must be symbol or string, got {}",
                other.type_name()
            )));
        }
    };
    match env.globals.lookup_var(&ns_name, &sym_name) {
        Some(var_ptr) => Ok(Value::Var(var_ptr)),
        None => Ok(Value::Nil),
    }
}

/// `(resolve sym)` — the Var for sym in `*ns*`, or nil.
pub fn eval_resolve(args: Vec<Value>, env: &mut Env) -> EvalResult {
    let [sym_arg] = args.as_slice() else {
        return Err(EvalError::Arity {
            name: "resolve".into(),
            expected: "1".into(),
            got: args.len(),
        });
    };
    let resolve_ns = resolve_current_ns(env);
    let sym_name = match sym_arg {
        Value::Symbol(s) => {
            let sym = s.get();
            // A qualified symbol resolves relative to `*ns*`, not to
            // `env.current_ns` — `resolve` is defined on the dynamic var.
            if let Some(ns) = &sym.namespace {
                let full_ns = env.globals.resolve_ns_part_in(&resolve_ns, ns.as_ref());
                return Ok(
                    match env.globals.lookup_var_in_ns(&full_ns, sym.name.as_ref()) {
                        Some(var_ptr) => Value::Var(var_ptr),
                        None => Value::Nil,
                    },
                );
            }
            sym.name.as_ref().to_string()
        }
        Value::Str(s) => s.get().clone(),
        other => {
            return Err(EvalError::Runtime(format!(
                "resolve: arg must be symbol or string, got {}",
                other.type_name()
            )));
        }
    };
    Ok(match env.globals.lookup_var_in_ns(&resolve_ns, &sym_name) {
        Some(var_ptr) => Value::Var(var_ptr),
        None => Value::Nil,
    })
}

/// `(intern ns sym)` / `(intern ns sym val)`.
pub fn eval_intern(args: Vec<Value>, env: &mut Env) -> EvalResult {
    if args.len() < 2 || args.len() > 3 {
        return Err(EvalError::Runtime("intern expects 2 or 3 arguments".into()));
    }
    let ns_name: Arc<str> = match &args[0] {
        Value::Symbol(s) => s.get().name.clone(),
        Value::Namespace(ns) => ns.get().name.clone(),
        other => {
            return Err(EvalError::Runtime(format!(
                "intern: first arg must be namespace or symbol, got {}",
                other.type_name()
            )));
        }
    };
    let var_name: Arc<str> = match &args[1] {
        Value::Symbol(s) => s.get().name.clone(),
        other => {
            return Err(EvalError::Runtime(format!(
                "intern: second arg must be symbol, got {}",
                other.type_name()
            )));
        }
    };
    // The namespace must already exist (Clojure throws otherwise).
    let ns = {
        let map = env.globals.namespaces.read().unwrap();
        map.get(ns_name.as_ref()).cloned()
    };
    let ns = ns.ok_or_else(|| EvalError::Runtime(format!("No namespace: {ns_name} found")))?;

    // Under no-gc: interned Vars are namespace-scoped and outlive every
    // scratch region.
    #[cfg(feature = "no-gc")]
    let _static_ctx = cljrs_gc::alloc_ctx::StaticCtxGuard::new();
    let mut interns = ns.get().interns.lock().unwrap();
    let var = match interns.get(&var_name) {
        Some(var) => var.clone(),
        None => {
            let var =
                cljrs_gc::GcPtr::new(cljrs_value::Var::new(ns_name.clone(), var_name.clone()));
            interns.insert(var_name, var.clone());
            var
        }
    };
    if let Some(val) = args.get(2) {
        var.get().bind(val.clone());
    }
    Ok(Value::Var(var))
}

/// `(bound-fn* f)` — capture the current dynamic bindings around `f`.
pub fn eval_bound_fn_star(args: Vec<Value>, _env: &mut Env) -> EvalResult {
    let [f] = args.as_slice() else {
        return Err(EvalError::Arity {
            name: "bound-fn*".into(),
            expected: "1".into(),
            got: args.len(),
        });
    };
    // Merge every binding frame into one flat frame, bottom-up so inner wins.
    let frames = crate::env::dynamics::capture_current();
    let mut merged = std::collections::HashMap::new();
    for frame in &frames {
        merged.extend(frame.iter().map(|(k, v)| (*k, v.clone())));
    }
    Ok(Value::BoundFn(cljrs_gc::GcPtr::new(cljrs_value::BoundFn {
        wrapped: f.clone(),
        captured_bindings: merged,
    })))
}
