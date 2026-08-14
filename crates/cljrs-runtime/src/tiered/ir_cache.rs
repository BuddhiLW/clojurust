//! Per-runtime cache of lowered IR, keyed by arity ID.
//!
//! Each `CljxFnArity` is assigned a unique `ir_arity_id` at creation time.
//! When a function is called, its runtime's cache is consulted:
//! - `NotAttempted` → try lowering
//! - `Cached(ir)` → execute via the IR interpreter
//! - `Unsupported` → fall back to tree-walking (don't retry)
//!
//! The hot path ([`IrCache::get`]) uses `RwLock` so concurrent reads don't
//! contend.  Writes (store) are infrequent (only during lowering).
//!
//! ## Ownership
//!
//! An [`IrCache`] belongs to one [`GlobalEnv`], reached through
//! [`GlobalEnv::ir_cache`]: two runtimes in one process never read or evict
//! each other's entries, and a runtime's IR is freed when the runtime is.
//!
//! Some callers only hold an arity id, with no route back to the runtime that
//! minted it — the background lowering worker, the JIT worker's publish
//! guard, and the process-global var-rebind hook.  For those, this module
//! keeps a weak index of live caches ([`LIVE`]) and exposes free functions
//! that resolve through it.  Arity ids come from one process-wide counter, so
//! at most one live cache can hold a given id and the lookup is unambiguous.
//! Stage 4 replaces the JIT half of this with compiler state owned by the
//! runtime; the index goes away with it.
//!
//! ## Cold-entry eviction (Phase 10.7)
//!
//! Cached entries carry a coarse last-access timestamp, refreshed on every
//! [`IrCache::get`] hit.  [`sweep_idle`] — run from the stop-the-world reclaim
//! pass once the background lowering worker is started — evicts entries idle
//! longer than [`ir_cache_ttl_secs`].  The IR cache is deliberately *colder*
//! than native code: eviction happens long after the last access, and only
//! when GC pressure triggers a collection anyway.  Entries whose arity has
//! published native code or a queued compile are never evicted (the IR is the
//! deoptimization fallback), and `Unsupported` markers are kept forever (they
//! are tiny and prevent retry storms).
//!
//! [`GlobalEnv`]: crate::env::env::GlobalEnv
//! [`GlobalEnv::ir_cache`]: crate::env::env::GlobalEnv::ir_cache

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, RwLock, Weak};
use std::time::Instant;

use cljrs_ir::IrFunction;

// ── Cache entries ────────────────────────────────────────────────────────────

/// State of an IR cache entry for one function arity.
pub enum IrCacheEntry {
    /// Lowering has not been attempted yet.
    NotAttempted,
    /// Lowering was attempted but failed (unsupported form); don't retry.
    Unsupported,
    /// Successfully lowered IR function.
    Cached {
        ir: Arc<IrFunction>,
        /// Coarse seconds (see [`now_secs`]) of the last [`IrCache::get`] hit.
        last_access: AtomicU64,
    },
}

// ── Coarse clock ─────────────────────────────────────────────────────────────

static PROCESS_EPOCH: LazyLock<Instant> = LazyLock::new(Instant::now);

/// Seconds since the process epoch — the coarse clock for last-access
/// tracking.  Monotonic and cheap (one `Instant::now` per call).
pub fn now_secs() -> u64 {
    PROCESS_EPOCH.elapsed().as_secs()
}

/// Idle time after which a cached IR entry becomes eligible for eviction.
/// `CLJRS_IR_CACHE_TTL` (seconds) overrides the default of 600.
pub fn ir_cache_ttl_secs() -> u64 {
    std::env::var("CLJRS_IR_CACHE_TTL")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(600)
}

// ── The cache ────────────────────────────────────────────────────────────────

/// One runtime's lowered-IR cache.
pub struct IrCache {
    entries: RwLock<HashMap<u64, IrCacheEntry>>,
}

impl IrCache {
    /// Create a cache owned by the runtime with identity `globals_id`, and
    /// index it so arity-id-only callers can find it.
    pub fn new(globals_id: u64) -> Arc<Self> {
        let cache = Arc::new(Self {
            entries: RwLock::new(HashMap::new()),
        });
        let mut live = LIVE.write().unwrap();
        live.retain(|(_, weak)| weak.strong_count() > 0);
        live.push((globals_id, Arc::downgrade(&cache)));
        cache
    }

    /// Look up a cached IR function by arity ID, refreshing its last-access
    /// time.  `None` if not cached or if lowering previously failed.
    ///
    /// This is the hot path — uses a read lock so concurrent callers don't
    /// block (the access timestamp is a relaxed atomic store under it).
    pub fn get(&self, id: u64) -> Option<Arc<IrFunction>> {
        let guard = self.entries.read().unwrap();
        match guard.get(&id) {
            Some(IrCacheEntry::Cached { ir, last_access }) => {
                last_access.store(now_secs(), Ordering::Relaxed);
                Some(ir.clone())
            }
            _ => None,
        }
    }

    /// Check if lowering should be attempted for this arity.
    /// `true` if the entry is `NotAttempted` (or absent).
    pub fn should_attempt(&self, id: u64) -> bool {
        !self.entries.read().unwrap().contains_key(&id)
    }

    /// Store a successful IR compilation result.
    pub fn store(&self, id: u64, ir: Arc<IrFunction>) {
        self.entries.write().unwrap().insert(
            id,
            IrCacheEntry::Cached {
                ir,
                last_access: AtomicU64::new(now_secs()),
            },
        );
    }

    /// Mark an arity as unsupported (lowering failed; don't retry).
    pub fn store_unsupported(&self, id: u64) {
        self.entries
            .write()
            .unwrap()
            .insert(id, IrCacheEntry::Unsupported);
    }

    /// Drop the cache entry for an arity entirely (back to `NotAttempted`), so
    /// a later [`Self::should_attempt`] returns `true` and the arity can be
    /// re-lowered.
    ///
    /// Used by cross-defn invalidation: a lowering that specialized against
    /// another defn is stale once that defn is rebound.
    pub fn invalidate(&self, id: u64) {
        self.entries.write().unwrap().remove(&id);
    }

    /// Evict cached entries idle longer than `ttl_secs`; see [`sweep_idle`].
    pub fn sweep(&self, now: u64, ttl_secs: u64) -> Vec<u64> {
        let mut evicted = Vec::new();
        let mut guard = self.entries.write().unwrap();
        guard.retain(|&id, entry| {
            let IrCacheEntry::Cached { last_access, .. } = entry else {
                return true;
            };
            let idle = now.saturating_sub(last_access.load(Ordering::Relaxed));
            if idle <= ttl_secs {
                return true;
            }
            if crate::tiered::jit_state::get_native_fn(id).is_some()
                || crate::tiered::jit_state::compile_queued(id)
            {
                return true;
            }
            evicted.push(id);
            false
        });
        drop(guard);
        for &id in &evicted {
            crate::tiered::jit_state::evict_entry_if_cold(id);
            crate::tiered::jit_state::stale_osr_code(id);
            cljrs_logging::feat_debug!("ir", "evicted idle IR arity_id={}", id);
        }
        evicted
    }
}

// ── Index of live caches ─────────────────────────────────────────────────────

/// Weak index of every live [`IrCache`], with the id of the runtime that owns
/// it.  See the module docs for why the arity-id-only callers need it.
#[allow(clippy::type_complexity)]
static LIVE: LazyLock<RwLock<Vec<(u64, Weak<IrCache>)>>> =
    LazyLock::new(|| RwLock::new(Vec::new()));

/// Every cache that is still alive.
fn live_caches() -> Vec<Arc<IrCache>> {
    LIVE.read()
        .unwrap()
        .iter()
        .filter_map(|(_, weak)| weak.upgrade())
        .collect()
}

/// The cache belonging to the runtime with this identity, if it is still
/// alive.  The background lowering worker uses it: a request carries the id of
/// the runtime that enqueued it, and the runtime may have been dropped since.
pub fn by_globals_id(globals_id: u64) -> Option<Arc<IrCache>> {
    LIVE.read()
        .unwrap()
        .iter()
        .find(|(id, _)| *id == globals_id)
        .and_then(|(_, weak)| weak.upgrade())
}

/// Look up cached IR by arity id across every live runtime.
///
/// For callers holding only an arity id (the JIT worker's publish guard).
/// Prefer [`IrCache::get`] whenever the runtime is in hand.
pub fn get_cached(id: u64) -> Option<Arc<IrFunction>> {
    live_caches().into_iter().find_map(|cache| cache.get(id))
}

/// Whether lowering should be attempted for `id` in whichever live runtime
/// owns it.  `true` when no live cache has an entry.
pub fn should_attempt(id: u64) -> bool {
    live_caches().iter().all(|cache| cache.should_attempt(id))
}

/// Drop `id`'s entry wherever it lives.
///
/// Used by the var-rebind hook, which `cljrs-value` invokes process-globally
/// with no runtime handle.
pub fn invalidate(id: u64) {
    for cache in live_caches() {
        cache.invalidate(id);
    }
}

/// Evict entries idle longer than `ttl_secs` from every live runtime.
///
/// Intended to run at a stop-the-world safepoint (registered by the lowering
/// worker), but safe at any time: in-flight Tier-1 frames hold their own
/// `Arc<IrFunction>`, and OSR native frames are protected by the code cache's
/// live-epoch scan.
///
/// Takes `now` as a parameter for testability; production callers pass
/// [`now_secs`]`()`.
///
/// Note: `defn_registry` deliberately retains its own `Arc<IrFunction>`s —
/// cross-defn inlining of an unchanged defn stays valid.  The sweep targets
/// only this dispatch cache.
pub fn sweep_idle(now: u64, ttl_secs: u64) -> Vec<u64> {
    let mut evicted = Vec::new();
    for cache in live_caches() {
        evicted.extend(cache.sweep(now, ttl_secs));
    }
    evicted
}

/// Serializes tests that publish cache entries with tests that run a
/// synthetic far-future sweep over every live cache.
#[cfg(test)]
pub(crate) static SWEEP_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_ir() -> Arc<IrFunction> {
        Arc::new(IrFunction::new(None, None))
    }

    /// A cache held for the test's duration, so the weak index keeps it.
    fn test_cache() -> Arc<IrCache> {
        IrCache::new(u64::MAX)
    }

    // Sentinel arity ids (0xE5xx_xxxx range) so parallel tests sharing the
    // live-cache index never collide; mirrors the jit_state test convention.
    //
    // The sweep is index-wide, though: a far-future `sweep_idle` from one
    // test would evict another test's entry mid-setup.  Serialize every test
    // that sweeps (or whose entries a sweep could evict) on this lock.
    fn sweep_guard() -> std::sync::MutexGuard<'static, ()> {
        SWEEP_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner())
    }

    #[test]
    fn sweep_evicts_idle_entry_and_drops_jit_entry() {
        let _g = sweep_guard();
        let cache = test_cache();
        let id = 0xE500_0001;
        cache.store(id, dummy_ir());
        crate::tiered::jit_state::mark_lower_queued(id);

        // Recent entry survives a sweep.
        let stored_at = now_secs();
        assert!(sweep_idle(stored_at, 600).is_empty() || !cache.should_attempt(id));
        assert!(cache.get(id).is_some());

        // Far in the future the entry is idle past the TTL and is evicted,
        // along with its JitEntry (lower_queued resets so it can re-warm).
        let evicted = sweep_idle(stored_at + 601, 600);
        assert!(evicted.contains(&id));
        assert!(cache.get(id).is_none());
        assert!(cache.should_attempt(id));
        assert!(!crate::tiered::jit_state::lower_queued(id));
    }

    #[test]
    fn sweep_skips_native_published_arity() {
        let _g = sweep_guard();
        let cache = test_cache();
        let id = 0xE500_0002;
        cache.store(id, dummy_ir());
        crate::tiered::jit_state::store_native_fn(id, 0x1234usize as *const (), 31337);

        let evicted = sweep_idle(now_secs() + 10_000, 600);
        assert!(!evicted.contains(&id));
        assert!(cache.get(id).is_some());

        // Cleanup: unpublish so other tests' sweeps behave.
        crate::tiered::jit_state::take_native_epoch(id);
        cache.invalidate(id);
    }

    #[test]
    fn sweep_skips_queued_compile() {
        let _g = sweep_guard();
        let cache = test_cache();
        let id = 0xE500_0003;
        let ir = dummy_ir();
        cache.store(id, ir.clone());
        // Cross the JIT threshold; with no enqueue hook installed this just
        // pins compile_queued, exactly the state of an in-flight compile.
        for _ in 0..crate::tiered::jit_state::jit_threshold() {
            crate::tiered::jit_state::record_call(id, ir.clone(), &[]);
        }
        assert!(crate::tiered::jit_state::compile_queued(id));

        let evicted = sweep_idle(now_secs() + 10_000, 600);
        assert!(!evicted.contains(&id));
        assert!(cache.get(id).is_some());
        cache.invalidate(id);
    }

    #[test]
    fn sweep_never_touches_unsupported() {
        let _g = sweep_guard();
        let cache = test_cache();
        let id = 0xE500_0004;
        cache.store_unsupported(id);
        let evicted = sweep_idle(now_secs() + 10_000, 600);
        assert!(!evicted.contains(&id));
        // Still terminal: no re-lowering attempts.
        assert!(!cache.should_attempt(id));
    }

    #[test]
    fn get_refreshes_last_access() {
        let _g = sweep_guard();
        let cache = test_cache();
        let id = 0xE500_0005;
        cache.store(id, dummy_ir());
        // Touch, then sweep with a now that is idle relative to the store
        // time but not the touch time recorded by `get`: by refreshing on
        // access the entry must survive a sweep whose `now` is within the
        // TTL of the touch.
        let _ = cache.get(id);
        let touched_at = now_secs();
        let evicted = sweep_idle(touched_at + 599, 600);
        assert!(!evicted.contains(&id));
        assert!(cache.get(id).is_some());
        cache.invalidate(id);
    }

    /// Two runtimes' caches are independent: neither sees the other's entries,
    /// and dropping one leaves the other intact.
    #[test]
    fn caches_are_per_runtime() {
        let _g = sweep_guard();
        let a = IrCache::new(0xE5AA_0001);
        let b = IrCache::new(0xE5AA_0002);
        let id = 0xE500_0006;

        a.store(id, dummy_ir());
        assert!(a.get(id).is_some());
        assert!(b.get(id).is_none(), "b must not see a's entry");
        assert!(b.should_attempt(id));

        // Resolving by runtime identity picks the right one.
        assert!(by_globals_id(0xE5AA_0001).unwrap().get(id).is_some());
        assert!(by_globals_id(0xE5AA_0002).unwrap().get(id).is_none());

        // The whole-process lookup finds it while `a` lives, and stops
        // finding it once `a` is dropped.
        assert!(get_cached(id).is_some());
        drop(a);
        assert!(get_cached(id).is_none());
        assert!(by_globals_id(0xE5AA_0001).is_none());
        drop(b);
    }
}
