#[cfg(not(feature = "no-gc"))]
use crate::env::dynamics;
use crate::env::env::Env;
#[cfg(not(feature = "no-gc"))]
use crate::env::env::GlobalEnv;
use std::cell::RefCell;

// ── Stop-the-world reclaim hooks (JIT code unloading, cold-IR sweep) ────────
//
// Reclamation of execution-engine caches runs only at a stop-the-world
// safepoint, when every mutator thread is parked and active JIT frames can be
// scanned safely.  GC collection is the existing STW point, so interested
// tiers install hooks here that run at the tail of every collection while the
// STW guard is still held.  Current registrants: the compiler's JIT code cache
// (`cljrs_compiler::jit::code_cache`, superseded native modules) and this
// crate's lowering worker (idle Tier-1 IR, Phase 10.7).

type StwReclaimHook = Box<dyn Fn() + Send + Sync + 'static>;
static STW_RECLAIM_HOOKS: std::sync::RwLock<Vec<StwReclaimHook>> =
    std::sync::RwLock::new(Vec::new());

/// Register a stop-the-world reclaim hook.  Multiple hooks may be registered;
/// each runs at every STW point, in registration order.
///
/// Hooks run inside the STW guard after each collection, so they may assume
/// all other mutator threads are parked.
pub fn set_stw_reclaim_hook(f: impl Fn() + Send + Sync + 'static) {
    STW_RECLAIM_HOOKS.write().unwrap().push(Box::new(f));
}

/// Run the STW reclaim hooks, if any.  Caller must hold the STW guard.
#[cfg(not(feature = "no-gc"))]
fn run_stw_reclaim() {
    for hook in STW_RECLAIM_HOOKS.read().unwrap().iter() {
        hook();
    }
}

// ── Thread-local Env root registry ──────────────────────────────────────────
//
// When the interpreter enters a function call, the caller's Env stays on the
// Rust stack but the callee creates a fresh Env.  If GC triggers inside the
// callee, only the callee's Env is passed to `gc_safepoint`.  To keep the
// caller's local bindings alive we maintain a thread-local stack of pointers
// to all active Envs on this thread's call stack.
//
// SAFETY: the raw pointers are valid during STW collection because:
// - The collecting thread's own Envs are in earlier (still-live) stack frames.
// - Other threads are parked at safepoints; their stacks (and Envs) are frozen.

thread_local! {
    static ENV_ROOTS: RefCell<Vec<*const Env>> = const { RefCell::new(Vec::new()) };
    /// Shadow stack of Value pointers on the Rust call stack that need to
    /// survive GC.  Each entry points to a contiguous slice of Values (e.g.,
    /// a Vec's backing storage or a single Value on the stack) and carries
    /// the id of the guard that owns it.
    static VALUE_ROOTS: RefCell<Vec<ValueRoot>> = const { RefCell::new(Vec::new()) };
    /// Source of [`ValueRoot::id`]s for this thread.
    static NEXT_VALUE_ROOT_ID: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// Shadow stack for `Option<Value>` slices (e.g., the IR interpreter's
    /// register file).  Each entry is `(ptr, count)` pointing to a fixed-size
    /// heap slice whose address will not change for the lifetime of the entry.
    static OPTION_VALUE_ROOTS: RefCell<Vec<(*const Option<cljrs_value::Value>, usize)>> =
        const { RefCell::new(Vec::new()) };
}

/// RAII guard that pops the Env pointer on drop.
pub struct EnvRootGuard;

impl Drop for EnvRootGuard {
    fn drop(&mut self) {
        ENV_ROOTS.with(|roots| {
            roots.borrow_mut().pop();
        });
    }
}

/// One entry of the value shadow stack.
struct ValueRoot {
    id: u64,
    ptr: *const cljrs_value::Value,
    len: usize,
}

/// RAII guard that removes its own entry from the value shadow stack on drop.
///
/// The guard borrows what it roots. The shadow stack holds a raw pointer to
/// those values, so they must neither move nor be freed while the guard
/// lives; the borrow makes the compiler reject a caller that hands a rooted
/// `Vec` to a callee, or pushes to it, with the guard still in scope.
///
/// Guards need not be dropped in the order they were made. Async tasks hold
/// them across `.await` and finish in any order, so a guard removes the entry
/// with its own id, not whichever entry is on top.
pub struct ValueRootGuard<'a> {
    id: Option<u64>,
    // The entry lives in this thread's shadow stack, so the guard must be
    // dropped on this thread: the raw pointer makes it `!Send`.
    _rooted: std::marker::PhantomData<(&'a [cljrs_value::Value], *const ())>,
}

impl Drop for ValueRootGuard<'_> {
    fn drop(&mut self) {
        let Some(id) = self.id else { return };
        VALUE_ROOTS.with(|roots| {
            let mut roots = roots.borrow_mut();
            // Almost always the newest entry.
            if let Some(at) = roots.iter().rposition(|root| root.id == id) {
                roots.remove(at);
            }
        });
    }
}

fn push_value_root<'a>(ptr: *const cljrs_value::Value, len: usize) -> ValueRootGuard<'a> {
    let id = NEXT_VALUE_ROOT_ID.with(|next| {
        let id = next.get();
        next.set(id.wrapping_add(1));
        id
    });
    VALUE_ROOTS.with(|roots| roots.borrow_mut().push(ValueRoot { id, ptr, len }));
    ValueRootGuard {
        id: Some(id),
        _rooted: std::marker::PhantomData,
    }
}

/// Register an Env as a GC root for the duration of its use.
/// Returns a guard that unregisters on drop.
pub fn push_env_root(env: &Env) -> EnvRootGuard {
    ENV_ROOTS.with(|roots| {
        roots.borrow_mut().push(env as *const Env);
    });
    EnvRootGuard
}

/// Register a single Value as a GC root.
pub fn root_value(val: &cljrs_value::Value) -> ValueRootGuard<'_> {
    push_value_root(val as *const cljrs_value::Value, 1)
}

/// Register a slice of Values as GC roots (e.g., a Vec<Value>).
///
/// The guard borrows the slice, so the values stay put while they are rooted:
///
/// ```
/// use cljrs_runtime::env::gc_roots::root_values;
/// let args = vec![cljrs_value::Value::Nil];
/// let root = root_values(&args);
/// drop(root);
/// drop(args);
/// ```
///
/// Handing the `Vec` away with the root still live does not compile. It used
/// to, and the collector then traced the freed buffer:
///
/// ```compile_fail,E0505
/// use cljrs_runtime::env::gc_roots::root_values;
/// let args = vec![cljrs_value::Value::Nil];
/// let root = root_values(&args);
/// drop(args);
/// drop(root);
/// ```
///
/// Neither does growing it, which may reallocate the buffer:
///
/// ```compile_fail,E0502
/// use cljrs_runtime::env::gc_roots::root_values;
/// let mut args = vec![cljrs_value::Value::Nil];
/// let root = root_values(&args);
/// args.push(cljrs_value::Value::Nil);
/// drop(root);
/// ```
pub fn root_values(vals: &[cljrs_value::Value]) -> ValueRootGuard<'_> {
    if vals.is_empty() {
        return ValueRootGuard {
            id: None,
            _rooted: std::marker::PhantomData,
        };
    }
    push_value_root(vals.as_ptr(), vals.len())
}

/// Register a slice of Values as GC roots without borrowing it, for an owner
/// that stores the guard next to the values it roots.
///
/// # Safety
///
/// The slice must stay at this address, with every element initialized, until
/// the returned guard is dropped: its storage must not be freed, reallocated
/// or moved. Elements may be overwritten in place. The guard must be dropped
/// before the storage is.
pub unsafe fn root_values_unchecked(vals: &[cljrs_value::Value]) -> ValueRootGuard<'static> {
    if vals.is_empty() {
        return ValueRootGuard {
            id: None,
            _rooted: std::marker::PhantomData,
        };
    }
    push_value_root(vals.as_ptr(), vals.len())
}

/// RAII guard that pops one entry from the option-value shadow stack on drop.
pub struct OptionValueRootGuard {
    pushed: bool,
}

impl Drop for OptionValueRootGuard {
    fn drop(&mut self) {
        if self.pushed {
            OPTION_VALUE_ROOTS.with(|roots| {
                roots.borrow_mut().pop();
            });
        }
    }
}

/// Register a slice of `Option<Value>` as GC roots.
///
/// The caller **must** ensure the slice's heap address is stable for the
/// lifetime of the returned guard — use `Box<[Option<Value>]>` rather than
/// a `Vec` that could reallocate.
pub fn root_option_values(vals: &[Option<cljrs_value::Value>]) -> OptionValueRootGuard {
    if vals.is_empty() {
        return OptionValueRootGuard { pushed: false };
    }
    OPTION_VALUE_ROOTS.with(|roots| {
        roots.borrow_mut().push((vals.as_ptr(), vals.len()));
    });
    OptionValueRootGuard { pushed: true }
}

/// Force an immediate GC collection, bypassing the memory-pressure threshold.
///
/// Unlike `gc_safepoint`, this always initiates collection regardless of
/// `gc_requested()`. Use this after removing namespaces from globals to ensure
/// their closures and form-trees are freed before the next namespace is loaded.
///
/// Under `no-gc` this is a no-op.
#[cfg(feature = "no-gc")]
pub fn force_collect(_env: &Env) {}

#[cfg(not(feature = "no-gc"))]
pub fn force_collect(env: &Env) {
    let Some(_stw_guard) = cljrs_gc::begin_stw() else {
        // Another thread is already collecting — just wait for it.
        cljrs_gc::safepoint();
        return;
    };

    cljrs_gc::HEAP.collect(|visitor| {
        cljrs_gc::HEAP.trace_registered_roots(visitor);
        trace_env_roots(env, visitor);
        trace_thread_env_roots(visitor);
        trace_value_roots(visitor);
        trace_option_value_roots(visitor);
        dynamics::trace_current(visitor);
        crate::env::taps::trace_roots(visitor);
        cljrs_gc::trace_thread_alloc_roots(visitor);
    });
    // Reclaim superseded JIT code while the world is still stopped.
    run_stw_reclaim();
}

/// Interpreter-level GC safepoint.
///
/// Under `no-gc` this is a no-op. Under GC mode it either parks (if collection
/// is in progress) or initiates a collection (if memory pressure was signalled).
#[cfg(feature = "no-gc")]
pub fn gc_safepoint(_env: &Env) {}

#[cfg(not(feature = "no-gc"))]
pub fn gc_safepoint(env: &Env) {
    // Fast path: no GC activity at all.
    if !cljrs_gc::gc_requested() && !cljrs_gc::CONFIG_CANCELLATION.in_progress() {
        return;
    }

    // If a GC is already in progress (another thread is collecting), just park.
    if cljrs_gc::CONFIG_CANCELLATION.in_progress() {
        cljrs_gc::safepoint();
        return;
    }

    // A GC was requested (memory pressure). Try to become the collector.
    if !cljrs_gc::take_gc_request() {
        // Another thread took the request; if collection started, park.
        cljrs_gc::safepoint();
        return;
    }

    // We won the request. Initiate STW collection.
    let Some(_stw_guard) = cljrs_gc::begin_stw() else {
        // Race: another thread started collecting between our take and begin.
        cljrs_gc::safepoint();
        return;
    };

    // All other threads are now parked. Collect with registered roots
    // plus ALL of this thread's active environments and dynamic bindings.
    cljrs_gc::HEAP.collect(|visitor| {
        // Trace globally registered roots (GlobalEnv, etc.)
        cljrs_gc::HEAP.trace_registered_roots(visitor);
        // Trace the current (innermost) env
        trace_env_roots(env, visitor);
        // Trace all caller Envs registered on this thread's stack
        trace_thread_env_roots(visitor);
        // Trace values on the Rust call stack (shadow stack)
        trace_value_roots(visitor);
        // Trace Option<Value> slices (e.g. IR interpreter register files)
        trace_option_value_roots(visitor);
        // Trace dynamic variable bindings on this thread
        dynamics::trace_current(visitor);
        // Trace the global tap system (functions and queued values)
        crate::env::taps::trace_roots(visitor);
        // Trace in-flight allocations from this thread's alloc root frames
        cljrs_gc::trace_thread_alloc_roots(visitor);
    });
    // Reclaim superseded JIT code while the world is still stopped.
    run_stw_reclaim();
    // _stw_guard drop clears in_progress, waking parked threads.
}

// ── GC-only root tracing helpers ─────────────────────────────────────────────

/// Trace all GcPtr values reachable from an Env's local frames.
#[cfg(not(feature = "no-gc"))]
fn trace_env_roots(env: &Env, visitor: &mut cljrs_gc::MarkVisitor) {
    use cljrs_gc::Trace;
    // Trace local frame bindings
    for frame in &env.frames {
        for (_name, val) in &frame.bindings {
            val.trace(visitor);
        }
    }
    // Trace the globals (namespaces, vars) — these are also registered
    // as root tracers, but it's safe to trace twice (idempotent marking).
    trace_globals(&env.globals, visitor);
}

/// Trace all Values registered in the thread-local value shadow stack.
#[cfg(not(feature = "no-gc"))]
fn trace_value_roots(visitor: &mut cljrs_gc::MarkVisitor) {
    use cljrs_gc::Trace;
    VALUE_ROOTS.with(|roots| {
        for root in roots.borrow().iter() {
            // SAFETY: each entry's guard either borrows the values it points
            // to, so they cannot have moved or been freed, or was made with
            // `root_values_unchecked`, whose caller vouches for the same.
            let slice = unsafe { std::slice::from_raw_parts(root.ptr, root.len) };
            for val in slice {
                val.trace(visitor);
            }
        }
    });
}

/// Trace all Option<Value> slices registered in the thread-local shadow stack.
///
/// Used for the IR interpreter's register file (a `Box<[Option<Value>]>`).
#[cfg(not(feature = "no-gc"))]
fn trace_option_value_roots(visitor: &mut cljrs_gc::MarkVisitor) {
    use cljrs_gc::Trace;
    OPTION_VALUE_ROOTS.with(|roots| {
        for &(ptr, count) in roots.borrow().iter() {
            // SAFETY: the slice is a Box<[Option<Value>]> owned by an active
            // stack frame; the address is stable for the guard's lifetime.
            let slice = unsafe { std::slice::from_raw_parts(ptr, count) };
            for val in slice.iter().flatten() {
                val.trace(visitor);
            }
        }
    });
}

/// Trace all Envs registered in the thread-local root stack.
#[cfg(not(feature = "no-gc"))]
fn trace_thread_env_roots(visitor: &mut cljrs_gc::MarkVisitor) {
    use cljrs_gc::Trace;
    ENV_ROOTS.with(|roots| {
        for env_ptr in roots.borrow().iter() {
            // SAFETY: pointers are valid — they point to Envs on this thread's
            // still-live stack frames (we are the collector, so our stack is active).
            let env = unsafe { &**env_ptr };
            for frame in &env.frames {
                for (_name, val) in &frame.bindings {
                    val.trace(visitor);
                }
            }
        }
    });
}

/// Trace all namespaces and their contents.
#[cfg(not(feature = "no-gc"))]
fn trace_globals(globals: &GlobalEnv, visitor: &mut cljrs_gc::MarkVisitor) {
    use cljrs_gc::{GcVisitor as _, Trace};
    let namespaces = globals.namespaces.read().unwrap();
    for ns_ptr in namespaces.values() {
        visitor.visit(ns_ptr);
    }
    drop(namespaces);
    // Values resolved at a pinned commit may live only in the version cache
    // (e.g. native HEAD fallbacks) — without this they would be collected.
    let version_cache = globals.version_cache.lock().unwrap();
    for val in version_cache.values() {
        val.trace(visitor);
    }
}

/// Service a pending GC request from an async (LocalSet) context.
///
/// Safe to call from within a Tokio `LocalSet` task at any cooperative yield
/// point: when this executes, no other tasks are polling, so thread-local root
/// stacks (ENV_ROOTS, VALUE_ROOTS, ALLOC_ROOTS) fully describe all GcPtrs held
/// by suspended tasks and can be scanned safely.
///
/// Under `no-gc` this is a no-op.
#[cfg(feature = "no-gc")]
pub fn async_gc_collect() {}

#[cfg(not(feature = "no-gc"))]
pub fn async_gc_collect() {
    if !cljrs_gc::gc_requested() && !cljrs_gc::CONFIG_CANCELLATION.in_progress() {
        return;
    }
    if cljrs_gc::CONFIG_CANCELLATION.in_progress() {
        cljrs_gc::safepoint();
        return;
    }
    if !cljrs_gc::take_gc_request() {
        cljrs_gc::safepoint();
        return;
    }
    let Some(_stw_guard) = cljrs_gc::begin_stw() else {
        cljrs_gc::safepoint();
        return;
    };
    cljrs_gc::HEAP.collect(|visitor| {
        cljrs_gc::HEAP.trace_registered_roots(visitor);
        trace_thread_env_roots(visitor);
        trace_value_roots(visitor);
        trace_option_value_roots(visitor);
        dynamics::trace_current(visitor);
        crate::env::taps::trace_roots(visitor);
        cljrs_gc::trace_thread_alloc_roots(visitor);
    });
    // Reclaim superseded JIT code while the world is still stopped.
    run_stw_reclaim();
}

#[cfg(test)]
mod tests {
    use super::*;
    use cljrs_value::Value;

    fn rooted() -> Vec<(*const Value, usize)> {
        VALUE_ROOTS.with(|roots| roots.borrow().iter().map(|r| (r.ptr, r.len)).collect())
    }

    #[test]
    fn a_guard_removes_its_own_entry_whatever_the_drop_order() {
        let before = rooted();
        let first = vec![Value::Nil];
        let second = vec![Value::Nil, Value::Nil];
        let first_root = root_values(&first);
        let second_root = root_values(&second);

        // Two async tasks finish in any order: the older guard goes first.
        drop(first_root);
        let mut expected = before.clone();
        expected.push((second.as_ptr(), 2));
        assert_eq!(rooted(), expected, "the newer root must survive");

        drop(second_root);
        assert_eq!(rooted(), before);
    }

    #[test]
    fn an_empty_slice_registers_nothing() {
        let before = rooted();
        let none: Vec<Value> = Vec::new();
        let root = root_values(&none);
        assert_eq!(rooted(), before);
        drop(root);
        assert_eq!(rooted(), before);
    }

    #[test]
    fn a_single_value_is_rooted_as_a_slice_of_one() {
        let before = rooted();
        let value = Value::Nil;
        let root = root_value(&value);
        let mut expected = before.clone();
        expected.push((&value as *const Value, 1));
        assert_eq!(rooted(), expected);
        drop(root);
        assert_eq!(rooted(), before);
    }
}
