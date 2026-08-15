//! Async-JIT activation (Phase H): compile `^:async` arities to native poll
//! functions on demand.
//!
//! `cljrs-async` drives `^:async` dispatch but cannot compile (it sits below
//! this crate).  It reaches this code through the runtime's `JitBackend`
//! ([`crate::jit::Jit::compile_async_arity`]), once per arity, the first time
//! that arity is invoked.  The call lowers the arity to a state
//! machine ([`cljrs_ir::lower::lower_async`]), JIT-compiles its poll function,
//! and registers it in `cljrs_async`'s poll-fn registry, after which dispatch
//! runs the native state machine.  Any unsupported construct (channels, spawn,
//! `throw`, …) makes lowering bail, leaving the arity on the `eval_async`
//! tree-walker.

use std::sync::Mutex;

use cljrs_async::state_machine::{PollFn, register_poll_fn};
use cljrs_runtime::env::env::Env;
use cljrs_runtime::interp::apply::select_arity;
use cljrs_value::Value;

use crate::jit::jit_compiler::{CompiledFn, compile_jit_poll};

/// Compiled poll-fn modules kept alive for the process lifetime: each registered
/// `poll_fn` pointer points into its module's executable memory.
///
/// These deliberately sit outside the epoch-tagged [`code_cache`](super::code_cache),
/// so nothing ever reclaims them: redefining an `^:async` var orphans the old
/// module, and *dropping the runtime that compiled it* orphans all of them,
/// along with their entries in `cljrs_async`'s process-global poll registry
/// (which is keyed by the process-unique `ir_arity_id`, so a later runtime
/// cannot collide with a dead one's entries — it is a leak, not a hazard).
/// Both are bounded by how much `^:async` code a process compiles.  Poll-fn
/// code unloading is future work: it needs the same epoch tagging and
/// live-frame scan the whole-function tier uses, plus a way to retract a
/// registry entry.
static KEEPALIVE: Mutex<Vec<CompiledFn>> = Mutex::new(Vec::new());

/// Lowers the called `^:async` arity to a state machine, JIT-compiles its
/// poll function, and
/// registers it under the arity's `ir_arity_id`.  A no-op on any lowering or
/// codegen failure, so the arity keeps tree-walking.
pub(crate) fn compile_async_arity(callee: &Value, nargs: usize, env: &mut Env) {
    let Value::Fn(f) = callee else { return };

    // Pull out everything the lowering needs, scoping the GcPtr borrow so it
    // ends before the `&mut env` lowering call below.
    let lowered = {
        let fr = f.get();
        // Capturing closures can't be lowered standalone — every captured local
        // would mis-resolve as a global — so keep them on the tree-walker (the
        // same restriction the warm-tier IR lowering enforces).
        if !fr.closed_over_names.is_empty() {
            return;
        }
        let Ok(arity) = select_arity(fr, nargs) else {
            return;
        };
        (
            arity.ir_arity_id,
            fr.name.clone(),
            fr.defining_ns.clone(),
            arity.params.clone(),
            arity.rest_param.clone(),
            arity.destructure_params.clone(),
            arity.destructure_rest.clone(),
            arity.body.clone(),
        )
    };
    let (arity_id, name, ns, params, rest_param, destructure_params, destructure_rest, body) =
        lowered;

    // Channel / concurrency ops (Phase H4) aren't modeled by the poll machine;
    // keep such fns on the tree-walker.
    if cljrs_ir::lower::async_lower::body_uses_unsupported_async(&body) {
        return;
    }

    // Form AST → IR with `is_async = true`.  We deliberately skip the
    // region-optimization pass: a bump-region scope (`RegionStart`/`RegionEnd`)
    // must not span a suspend, since the poll function returns to the executor
    // mid-scope and would leave the region stack unbalanced.  Async bodies use
    // the GC heap; region promotion for them is future work (close regions
    // before a suspend, reopen on resume).
    let ir = match cljrs_runtime::tiered::lower::lower_arity(
        name.as_deref(),
        &params,
        rest_param.as_ref(),
        &destructure_params,
        destructure_rest.as_ref(),
        &body,
        &ns,
        env,
        true,
    ) {
        Ok(ir) => ir,
        Err(_) => return,
    };

    // IR → state-machine poll function (bails on unsupported constructs).
    let low = match cljrs_ir::lower::lower_async(&ir) {
        Ok(low) => low,
        Err(_) => return,
    };

    let sym = format!("__cljrs_async_poll_{arity_id}");
    let cf = match compile_jit_poll(&sym, &low.poll_fn) {
        Ok(cf) => cf,
        Err(_) => return,
    };

    // SAFETY: `compile_jit_poll` declared the poll-fn ABI for this symbol, and
    // `cf` (kept alive below) owns the executable memory `fn_ptr` points into.
    let poll_fn: PollFn = unsafe { std::mem::transmute::<*const (), PollFn>(cf.fn_ptr) };
    KEEPALIVE.lock().unwrap().push(cf);
    register_poll_fn(arity_id, poll_fn, low.n_slots);
}
