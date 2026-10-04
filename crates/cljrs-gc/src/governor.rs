//! Process-wide managed-memory governor.
//!
//! See `docs/managed-memory-governor-plan.md`.  Every isolate keeps its private
//! heap and collects it independently; the governor owns only counters,
//! pressure policy, and thread-safe [`IsolateControl`] handles.
//!
//! **Status: Phase 1 (observe-only).**  The governor accounts GC heap bytes per
//! isolate, derives a process [`PressureLevel`], and requests collection from
//! isolates at `Yellow`/`Red`.  It never rejects an allocation: when committed
//! bytes pass the hard limit it records a *would-reject* event instead (see
//! [`MemorySnapshot::over_limit_events`]).
//!
//! The hot path ([`IsolateAccount::charge`]) touches only thread-local `Cell`s.
//! An account publishes its byte count to the process counters once it has
//! drifted by one credit chunk ([`MemoryConfig::credit_chunk`]) from the last
//! published value, and after every collection.  Process totals therefore lag
//! each isolate by less than one chunk.

use std::cell::{Cell, OnceCell};
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

const KIB: usize = 1024;
const MIB: usize = 1024 * 1024;

/// Default isolate publication granularity (the future credit chunk).
pub const DEFAULT_CREDIT_CHUNK: usize = 64 * KIB;
/// Floor for the default critical reserve.
pub const MIN_DEFAULT_CRITICAL_RESERVE: usize = 4 * MIB;
/// Ceiling for the default critical reserve.
pub const MAX_DEFAULT_CRITICAL_RESERVE: usize = 256 * MIB;
/// Ceiling for the default per-channel queue limit.
pub const DEFAULT_QUEUE_LIMIT: usize = 64 * MIB;

/// Process-wide soft limit, in MiB.
pub const SOFT_LIMIT_ENV: &str = "CLJRS_MEMORY_SOFT_LIMIT_MB";
/// Process-wide hard limit, in MiB.
pub const HARD_LIMIT_ENV: &str = "CLJRS_MEMORY_HARD_LIMIT_MB";
/// Critical reserve for runtime control operations, in MiB.
pub const CRITICAL_RESERVE_ENV: &str = "CLJRS_MEMORY_CRITICAL_RESERVE_MB";
/// Isolate credit chunk, in KiB.
pub const CREDIT_ENV: &str = "CLJRS_MEMORY_CREDIT_KB";
/// Default byte limit for one isolate channel, in MiB.
pub const QUEUE_LIMIT_ENV: &str = "CLJRS_ISOLATE_QUEUE_LIMIT_MB";
/// Deprecated alias for [`SOFT_LIMIT_ENV`].
pub const DEPRECATED_SOFT_LIMIT_ENV: &str = "CLJRS_GC_SOFT_LIMIT_MB";
/// Deprecated alias for [`HARD_LIMIT_ENV`].
pub const DEPRECATED_HARD_LIMIT_ENV: &str = "CLJRS_GC_HARD_LIMIT_MB";

// ── Memory classes and pressure ──────────────────────────────────────────────

/// Owner and lifetime rule for a governed allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemoryClass {
    /// Objects in an isolate's private tracing heap.
    GcHeap,
    /// Bump-region chunks (active and retired).
    Region,
    /// Shared immutable values (`ByteBlob`, `SharedAtom` storage, …).
    SharedValue,
    /// Serialized messages queued on isolate channels.
    MessageQueue,
    /// Program-lifetime static allocations.
    Static,
    /// JIT code and code caches.
    Code,
    /// Other runtime-owned data.
    Runtime,
}

const CLASS_COUNT: usize = 7;

impl MemoryClass {
    /// Every memory class, in reporting order.
    pub const ALL: [MemoryClass; CLASS_COUNT] = [
        MemoryClass::GcHeap,
        MemoryClass::Region,
        MemoryClass::SharedValue,
        MemoryClass::MessageQueue,
        MemoryClass::Static,
        MemoryClass::Code,
        MemoryClass::Runtime,
    ];

    /// Static, allocation-free name used in diagnostics and errors.
    pub const fn name(self) -> &'static str {
        match self {
            MemoryClass::GcHeap => "gc-heap",
            MemoryClass::Region => "region",
            MemoryClass::SharedValue => "shared-value",
            MemoryClass::MessageQueue => "message-queue",
            MemoryClass::Static => "static",
            MemoryClass::Code => "code",
            MemoryClass::Runtime => "runtime",
        }
    }

    const fn index(self) -> usize {
        self as usize
    }
}

/// Process memory pressure, derived from committed bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum PressureLevel {
    /// Below the soft limit.
    Green = 0,
    /// At or above the soft limit.
    Yellow = 1,
    /// Above the hard limit (strict mode would reject new application credit).
    Red = 2,
}

impl PressureLevel {
    fn from_u8(v: u8) -> Self {
        match v {
            0 => PressureLevel::Green,
            1 => PressureLevel::Yellow,
            _ => PressureLevel::Red,
        }
    }

    /// Classify `committed` bytes against the given limits.
    pub fn classify(committed: usize, soft_limit: usize, hard_limit: usize) -> Self {
        if committed > hard_limit {
            PressureLevel::Red
        } else if committed >= soft_limit {
            PressureLevel::Yellow
        } else {
            PressureLevel::Green
        }
    }
}

impl fmt::Display for PressureLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            PressureLevel::Green => "Green",
            PressureLevel::Yellow => "Yellow",
            PressureLevel::Red => "Red",
        })
    }
}

// ── Configuration ────────────────────────────────────────────────────────────

/// Process-wide managed-memory limits.  Read once per process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryConfig {
    /// Committed bytes at which pressure becomes `Yellow`.
    pub soft_limit: usize,
    /// Committed bytes above which pressure becomes `Red`.
    pub hard_limit: usize,
    /// Capacity reserved for collection, shutdown, and error reporting.
    pub critical_reserve: usize,
    /// Isolate credit chunk (Phase 1: publication granularity).
    pub credit_chunk: usize,
    /// Default byte limit for one isolate channel.
    pub queue_limit: usize,
}

/// A rejected [`MemoryConfig`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryConfigError {
    /// The hard limit is zero.
    ZeroHardLimit,
    /// The soft limit is greater than the hard limit.
    SoftAboveHard {
        soft_limit: usize,
        hard_limit: usize,
    },
    /// The critical reserve is greater than the hard limit.
    ReserveAboveHard {
        critical_reserve: usize,
        hard_limit: usize,
    },
    /// The credit chunk is zero.
    ZeroCreditChunk,
    /// The queue limit is greater than the application budget.
    QueueAboveBudget {
        queue_limit: usize,
        hard_limit: usize,
    },
    /// An environment variable is not a non-negative integer.
    InvalidNumber { var: &'static str, value: String },
}

impl fmt::Display for MemoryConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MemoryConfigError::ZeroHardLimit => write!(f, "the hard memory limit must be non-zero"),
            MemoryConfigError::SoftAboveHard {
                soft_limit,
                hard_limit,
            } => write!(
                f,
                "the soft memory limit ({soft_limit} bytes) is greater than the hard limit ({hard_limit} bytes)"
            ),
            MemoryConfigError::ReserveAboveHard {
                critical_reserve,
                hard_limit,
            } => write!(
                f,
                "the critical reserve ({critical_reserve} bytes) is greater than the hard limit ({hard_limit} bytes)"
            ),
            MemoryConfigError::ZeroCreditChunk => write!(f, "the credit chunk must be non-zero"),
            MemoryConfigError::QueueAboveBudget {
                queue_limit,
                hard_limit,
            } => write!(
                f,
                "the isolate queue limit ({queue_limit} bytes) is greater than the hard limit ({hard_limit} bytes)"
            ),
            MemoryConfigError::InvalidNumber { var, value } => {
                write!(f, "{var}={value:?} is not a non-negative integer")
            }
        }
    }
}

impl std::error::Error for MemoryConfigError {}

impl MemoryConfig {
    /// Build a config from explicit limits; the remaining fields take their
    /// defaults for `hard_limit`.
    pub fn with_limits(soft_limit: usize, hard_limit: usize) -> Result<Self, MemoryConfigError> {
        let config = Self {
            soft_limit,
            hard_limit,
            critical_reserve: default_critical_reserve(hard_limit),
            credit_chunk: DEFAULT_CREDIT_CHUNK,
            queue_limit: DEFAULT_QUEUE_LIMIT.min(hard_limit),
        };
        config.validate()?;
        Ok(config)
    }

    /// Build a config from optional soft and hard limits.
    ///
    /// A missing hard limit uses [`default_hard_limit`], raised to the soft
    /// limit when the soft limit is larger.  A missing soft limit is 75% of
    /// the hard limit.
    pub fn from_optional_limits(
        soft_limit: Option<usize>,
        hard_limit: Option<usize>,
    ) -> Result<Self, MemoryConfigError> {
        let hard = match (soft_limit, hard_limit) {
            (_, Some(hard)) => hard,
            (Some(soft), None) => default_hard_limit().max(soft),
            (None, None) => default_hard_limit(),
        };
        let soft = soft_limit.unwrap_or_else(|| default_soft_limit(hard));
        Self::with_limits(soft, hard)
    }

    /// Platform defaults: container limit or a fraction of physical memory.
    pub fn platform_default() -> Self {
        let hard = default_hard_limit().max(1);
        Self::with_limits(default_soft_limit(hard), hard)
            .expect("platform default memory config is valid")
    }

    /// Check the invariants the plan requires of a config.
    pub fn validate(&self) -> Result<(), MemoryConfigError> {
        if self.hard_limit == 0 {
            return Err(MemoryConfigError::ZeroHardLimit);
        }
        if self.soft_limit > self.hard_limit {
            return Err(MemoryConfigError::SoftAboveHard {
                soft_limit: self.soft_limit,
                hard_limit: self.hard_limit,
            });
        }
        if self.critical_reserve > self.hard_limit {
            return Err(MemoryConfigError::ReserveAboveHard {
                critical_reserve: self.critical_reserve,
                hard_limit: self.hard_limit,
            });
        }
        if self.credit_chunk == 0 {
            return Err(MemoryConfigError::ZeroCreditChunk);
        }
        if self.queue_limit > self.hard_limit {
            return Err(MemoryConfigError::QueueAboveBudget {
                queue_limit: self.queue_limit,
                hard_limit: self.hard_limit,
            });
        }
        Ok(())
    }

    /// Read the process config from the environment.
    ///
    /// The deprecated `CLJRS_GC_*_LIMIT_MB` names set process-wide values; a
    /// new variable wins over its old alias.  Each old variable in use
    /// produces one warning on stderr.
    pub fn from_env() -> Result<Self, MemoryConfigError> {
        Self::from_lookup(|var| std::env::var(var).ok(), |msg| eprintln!("{msg}"))
    }

    /// [`Self::from_env`] over an arbitrary variable lookup (for tests).
    pub fn from_lookup(
        lookup: impl Fn(&str) -> Option<String>,
        mut warn: impl FnMut(String),
    ) -> Result<Self, MemoryConfigError> {
        let mut limit = |new: &'static str, old: &'static str| -> Result<Option<usize>, _> {
            let new_val = parse_env(new, lookup(new), MIB)?;
            if let Some(old_val) = lookup(old) {
                if new_val.is_some() {
                    warn(format!(
                        "[memory] warning: {old} is deprecated and ignored because {new} is set"
                    ));
                    return Ok(new_val);
                }
                warn(format!(
                    "[memory] warning: {old} is deprecated; use {new} (it sets a process-wide limit)"
                ));
                return parse_env(old, Some(old_val), MIB);
            }
            Ok(new_val)
        };
        let soft = limit(SOFT_LIMIT_ENV, DEPRECATED_SOFT_LIMIT_ENV)?;
        let hard = limit(HARD_LIMIT_ENV, DEPRECATED_HARD_LIMIT_ENV)?;
        let mut config = Self::from_optional_limits(soft, hard)?;
        if let Some(reserve) = parse_env(CRITICAL_RESERVE_ENV, lookup(CRITICAL_RESERVE_ENV), MIB)? {
            config.critical_reserve = reserve;
        }
        if let Some(chunk) = parse_env(CREDIT_ENV, lookup(CREDIT_ENV), KIB)? {
            config.credit_chunk = chunk;
        }
        if let Some(queue) = parse_env(QUEUE_LIMIT_ENV, lookup(QUEUE_LIMIT_ENV), MIB)? {
            config.queue_limit = queue;
        }
        config.validate()?;
        Ok(config)
    }
}

fn parse_env(
    var: &'static str,
    value: Option<String>,
    unit: usize,
) -> Result<Option<usize>, MemoryConfigError> {
    match value {
        None => Ok(None),
        Some(raw) => raw
            .trim()
            .parse::<usize>()
            .map(|n| Some(n.saturating_mul(unit)))
            .map_err(|_| MemoryConfigError::InvalidNumber { var, value: raw }),
    }
}

/// Default soft limit: 75% of the hard limit.
pub fn default_soft_limit(hard_limit: usize) -> usize {
    // Round up, so a nonzero hard limit never yields a zero soft limit
    // (which would hold the process at `Yellow` permanently).
    hard_limit - hard_limit / 4
}

/// Default critical reserve: the larger of 4 MiB or 1% of the hard limit,
/// capped at [`MAX_DEFAULT_CRITICAL_RESERVE`] and at the hard limit itself.
pub fn default_critical_reserve(hard_limit: usize) -> usize {
    (hard_limit / 100)
        .clamp(MIN_DEFAULT_CRITICAL_RESERVE, MAX_DEFAULT_CRITICAL_RESERVE)
        .min(hard_limit)
}

/// Default process hard limit.
///
/// Uses the cgroup memory limit when one applies (Linux), otherwise half of
/// physical memory.  `wasm32` uses a fixed 256 MiB.
///
/// This is the process-wide budget.  It is distinct from the per-heap
/// default in `config.rs` (¼ of RAM, at least 256 MiB), which only sets each
/// isolate heap's unenforced hard limit, so the two may differ.
pub fn default_hard_limit() -> usize {
    #[cfg(target_arch = "wasm32")]
    {
        256 * MIB
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let physical = system_memory::total() as usize;
        let half = (physical / 2).max(256 * MIB);
        match container_memory_limit() {
            Some(limit) if limit < physical || physical == 0 => limit,
            _ => half,
        }
    }
}

/// The cgroup (v2, then v1) memory limit of this process, if any.
#[cfg(not(target_arch = "wasm32"))]
pub fn container_memory_limit() -> Option<usize> {
    #[cfg(target_os = "linux")]
    {
        const PATHS: [&str; 2] = [
            "/sys/fs/cgroup/memory.max",
            "/sys/fs/cgroup/memory/memory.limit_in_bytes",
        ];
        PATHS.iter().find_map(|path| {
            let raw = std::fs::read_to_string(path).ok()?;
            let raw = raw.trim();
            if raw == "max" {
                return None;
            }
            // cgroup v1 reports "unlimited" as a value near i64::MAX.
            raw.parse::<u64>()
                .ok()
                .filter(|&n| n > 0 && n < (1u64 << 60))
                .map(|n| n as usize)
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

// ── Isolate control ──────────────────────────────────────────────────────────

/// Process-unique isolate identifier.
pub type IsolateId = u64;

/// Thread-safe control data for one registered isolate.
///
/// The governor holds weak references to these in its registry; it can
/// request collection, but it cannot trace or sweep another isolate's heap.
pub struct IsolateControl {
    id: IsolateId,
    name: Mutex<Arc<str>>,
    gc_requested: AtomicBool,
    used_bytes: AtomicUsize,
    peak_used_bytes: AtomicUsize,
    /// Number of completed collections; the collection epoch.
    last_collection_epoch: AtomicU64,
    /// Epoch at which the governor last requested collection (`u64::MAX`: never).
    requested_epoch: AtomicU64,
    /// Used bytes below which the governor does not request collection at
    /// `Yellow` (set from the last collection's result).
    collection_target: AtomicUsize,
    collection_requests: AtomicU64,
    zero_yield_collections: AtomicU64,
    last_bytes_before: AtomicUsize,
    last_bytes_after: AtomicUsize,
    last_duration_nanos: AtomicU64,
}

impl IsolateControl {
    fn new(id: IsolateId, name: Arc<str>) -> Self {
        Self {
            id,
            name: Mutex::new(name),
            gc_requested: AtomicBool::new(false),
            used_bytes: AtomicUsize::new(0),
            peak_used_bytes: AtomicUsize::new(0),
            last_collection_epoch: AtomicU64::new(0),
            requested_epoch: AtomicU64::new(u64::MAX),
            collection_target: AtomicUsize::new(0),
            collection_requests: AtomicU64::new(0),
            zero_yield_collections: AtomicU64::new(0),
            last_bytes_before: AtomicUsize::new(0),
            last_bytes_after: AtomicUsize::new(0),
            last_duration_nanos: AtomicU64::new(0),
        }
    }

    /// Process-unique id.
    pub fn id(&self) -> IsolateId {
        self.id
    }

    /// Diagnostic name.
    pub fn name(&self) -> Arc<str> {
        self.name
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn set_name(&self, name: Arc<str>) {
        *self.name.lock().unwrap_or_else(PoisonError::into_inner) = name;
    }

    /// Ask the isolate to collect at its next safepoint.
    pub fn request_collection(&self) {
        self.gc_requested.store(true, Ordering::Release);
    }

    /// Whether a collection request is pending.
    pub fn collection_requested(&self) -> bool {
        self.gc_requested.load(Ordering::Acquire)
    }

    /// Check and clear the pending collection request.
    pub fn take_collection_request(&self) -> bool {
        self.gc_requested.swap(false, Ordering::AcqRel)
    }

    /// Last published used bytes.
    pub fn used_bytes(&self) -> usize {
        self.used_bytes.load(Ordering::Relaxed)
    }

    /// Number of completed collections.
    pub fn collection_epoch(&self) -> u64 {
        self.last_collection_epoch.load(Ordering::Relaxed)
    }

    /// Governor-initiated collection requests for this isolate.
    pub fn collection_requests(&self) -> u64 {
        self.collection_requests.load(Ordering::Relaxed)
    }

    /// Request collection on the governor's behalf, at most once per
    /// collection epoch.  Below `Red`, also skip the request while the heap
    /// is under its post-collection target.  Returns whether a request was
    /// made.
    fn governor_request(&self, level: PressureLevel) -> bool {
        let epoch = self.collection_epoch();
        let prev = self.requested_epoch.load(Ordering::Relaxed);
        if prev == epoch {
            return false;
        }
        if level < PressureLevel::Red
            && self.used_bytes() < self.collection_target.load(Ordering::Relaxed)
        {
            return false;
        }
        // `evaluate` can run on several publishing threads at once; only the
        // thread that claims this epoch counts the request.
        if self
            .requested_epoch
            .compare_exchange(prev, epoch, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
        {
            return false;
        }
        self.collection_requests.fetch_add(1, Ordering::Relaxed);
        self.request_collection();
        true
    }

    fn snapshot(&self) -> IsolateSnapshot {
        IsolateSnapshot {
            id: self.id,
            name: self.name(),
            used_bytes: self.used_bytes(),
            peak_used_bytes: self.peak_used_bytes.load(Ordering::Relaxed),
            collections: self.collection_epoch(),
            collection_requests: self.collection_requests(),
            zero_yield_collections: self.zero_yield_collections.load(Ordering::Relaxed),
            last_bytes_before: self.last_bytes_before.load(Ordering::Relaxed),
            last_bytes_after: self.last_bytes_after.load(Ordering::Relaxed),
            last_duration: Duration::from_nanos(self.last_duration_nanos.load(Ordering::Relaxed)),
        }
    }
}

impl fmt::Debug for IsolateControl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IsolateControl")
            .field("id", &self.id)
            .field("name", &self.name())
            .field("used_bytes", &self.used_bytes())
            .finish()
    }
}

/// What one collection did, reported to the isolate's account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CollectionReport {
    /// Heap bytes before collection.
    pub bytes_before: usize,
    /// Heap bytes after collection.
    pub bytes_after: usize,
    /// Bytes freed by sweep and returned to the governor.
    pub bytes_returned: usize,
    /// Mark plus sweep time.
    pub duration: Duration,
}

// ── Isolate account ──────────────────────────────────────────────────────────

/// One isolate's single-threaded view of the governor.
///
/// The counters are `Cell`s, so the account is `!Sync`: only one thread
/// uses it at a time.  It is `Send`, so nothing stops moving it to another
/// thread; the runtime keeps it in a thread-local for the isolate's thread.
///
/// Created by [`ProcessMemoryGovernor::register_isolate`].  Dropping the
/// account returns its charge and unregisters it.
pub struct IsolateAccount {
    governor: &'static ProcessMemoryGovernor,
    control: Arc<IsolateControl>,
    used: Cell<usize>,
    published: Cell<usize>,
    publish_step: Cell<usize>,
}

impl IsolateAccount {
    /// Shared control handle.
    pub fn control(&self) -> &Arc<IsolateControl> {
        &self.control
    }

    /// Exact local used bytes (may be ahead of the published value).
    pub fn used_bytes(&self) -> usize {
        self.used.get()
    }

    /// Account `bytes` of new heap allocation.  Thread-local unless the local
    /// value has drifted one credit chunk from the published value.
    #[inline]
    pub fn charge(&self, bytes: usize) {
        let used = self.used.get() + bytes;
        self.used.set(used);
        // Drift since the last publish.  `published` can exceed `used`
        // between a sweep's releases and the next publish; `min` keeps the
        // subtraction from underflowing in that window.
        if used - self.published.get().min(used) >= self.publish_step.get() {
            self.publish();
        }
    }

    /// Return `bytes` freed by sweep.
    #[inline]
    pub fn release(&self, bytes: usize) {
        let used = self.used.get();
        debug_assert!(
            bytes <= used,
            "isolate {} released {bytes} bytes with only {used} charged",
            self.control.id
        );
        self.used.set(used.saturating_sub(bytes));
    }

    /// Push the local byte count to the process counters and re-evaluate
    /// pressure.
    pub fn publish(&self) {
        let used = self.used.get();
        let prev = self.published.replace(used);
        self.control.used_bytes.store(used, Ordering::Relaxed);
        self.control
            .peak_used_bytes
            .fetch_max(used, Ordering::Relaxed);
        self.publish_step.set(self.governor.credit_chunk());
        if used >= prev {
            self.governor.add(MemoryClass::GcHeap, used - prev);
        } else {
            self.governor.sub(MemoryClass::GcHeap, prev - used);
        }
        self.governor.evaluate(Some(&self.control), used > prev);
    }

    /// Record a completed collection: bump the epoch, set the next
    /// collection target, and publish.
    pub fn record_collection(&self, report: CollectionReport) {
        let c = &self.control;
        c.last_collection_epoch.fetch_add(1, Ordering::Relaxed);
        c.last_bytes_before
            .store(report.bytes_before, Ordering::Relaxed);
        c.last_bytes_after
            .store(report.bytes_after, Ordering::Relaxed);
        c.last_duration_nanos.store(
            report.duration.as_nanos().min(u64::MAX as u128) as u64,
            Ordering::Relaxed,
        );
        let min_headroom = self.governor.credit_chunk().saturating_mul(16).max(MIB);
        let headroom = if report.bytes_returned == 0 {
            c.zero_yield_collections.fetch_add(1, Ordering::Relaxed);
            report.bytes_after.max(min_headroom)
        } else {
            (report.bytes_after / 2).max(min_headroom)
        };
        c.collection_target.store(
            report.bytes_after.saturating_add(headroom),
            Ordering::Relaxed,
        );
        self.publish();
    }
}

impl Drop for IsolateAccount {
    fn drop(&mut self) {
        // Phase 1: the isolate heap is not torn down at thread exit, so this
        // stops counting bytes that remain allocated until process exit.
        let published = self.published.replace(0);
        self.used.set(0);
        self.control.used_bytes.store(0, Ordering::Relaxed);
        self.governor.sub(MemoryClass::GcHeap, published);
        self.governor.unregister(&self.control);
        self.governor.evaluate(None, false);
    }
}

// ── Charges for independent-lifetime allocations ─────────────────────────────

/// RAII charge for an allocation with its own lifetime.  Dropping it returns
/// the bytes to the governor exactly once.
#[must_use = "dropping a MemoryCharge returns its bytes immediately"]
pub struct MemoryCharge {
    governor: &'static ProcessMemoryGovernor,
    class: MemoryClass,
    bytes: usize,
}

impl MemoryCharge {
    /// Charged class.
    pub fn class(&self) -> MemoryClass {
        self.class
    }

    /// Charged bytes.
    pub fn bytes(&self) -> usize {
        self.bytes
    }
}

impl Drop for MemoryCharge {
    fn drop(&mut self) {
        self.governor.sub(self.class, self.bytes);
        self.governor.evaluate(None, false);
    }
}

impl fmt::Debug for MemoryCharge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemoryCharge")
            .field("class", &self.class)
            .field("bytes", &self.bytes)
            .finish()
    }
}

/// A reservation the governor refused.  Fixed-size; building it never
/// allocates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryLimitExceeded {
    /// Class of the refused reservation.
    pub class: MemoryClass,
    /// Requested bytes.
    pub requested: usize,
    /// Committed bytes at the time of the request.
    pub committed: usize,
    /// Hard limit at the time of the request.
    pub hard_limit: usize,
}

impl fmt::Display for MemoryLimitExceeded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "managed memory limit exceeded: {} bytes of {} requested with {} of {} bytes committed",
            self.requested,
            self.class.name(),
            self.committed,
            self.hard_limit
        )
    }
}

impl std::error::Error for MemoryLimitExceeded {}

// ── Process governor ─────────────────────────────────────────────────────────

/// Process-wide managed-memory governor.  See the module docs.
pub struct ProcessMemoryGovernor {
    config: Mutex<Option<MemoryConfig>>,
    configured: AtomicBool,
    soft_limit: AtomicUsize,
    hard_limit: AtomicUsize,
    credit_chunk: AtomicUsize,
    class_bytes: [AtomicUsize; CLASS_COUNT],
    peak_committed: AtomicUsize,
    pressure: AtomicU8,
    pressure_transitions: AtomicU64,
    collection_requests: AtomicU64,
    over_limit_events: AtomicU64,
    over_limit_bytes: AtomicU64,
    registry: Mutex<Vec<Weak<IsolateControl>>>,
    next_isolate_id: AtomicU64,
    isolates_registered: AtomicU64,
}

impl ProcessMemoryGovernor {
    /// A governor that reads [`MemoryConfig::from_env`] on first use.
    pub const fn new() -> Self {
        Self {
            config: Mutex::new(None),
            configured: AtomicBool::new(false),
            soft_limit: AtomicUsize::new(usize::MAX),
            hard_limit: AtomicUsize::new(usize::MAX),
            credit_chunk: AtomicUsize::new(DEFAULT_CREDIT_CHUNK),
            class_bytes: [const { AtomicUsize::new(0) }; CLASS_COUNT],
            peak_committed: AtomicUsize::new(0),
            pressure: AtomicU8::new(PressureLevel::Green as u8),
            pressure_transitions: AtomicU64::new(0),
            collection_requests: AtomicU64::new(0),
            over_limit_events: AtomicU64::new(0),
            over_limit_bytes: AtomicU64::new(0),
            registry: Mutex::new(Vec::new()),
            next_isolate_id: AtomicU64::new(1),
            isolates_registered: AtomicU64::new(0),
        }
    }

    /// A governor with an explicit config.
    pub fn with_config(config: MemoryConfig) -> Result<Self, MemoryConfigError> {
        let g = Self::new();
        g.configure(config)?;
        Ok(g)
    }

    /// Replace the process limits and re-evaluate pressure.
    pub fn configure(&self, config: MemoryConfig) -> Result<(), MemoryConfigError> {
        config.validate()?;
        let mut slot = self.config.lock().unwrap_or_else(PoisonError::into_inner);
        self.install(&config);
        *slot = Some(config);
        drop(slot);
        self.evaluate(None, false);
        Ok(())
    }

    fn install(&self, config: &MemoryConfig) {
        self.soft_limit.store(config.soft_limit, Ordering::Relaxed);
        self.hard_limit.store(config.hard_limit, Ordering::Relaxed);
        self.credit_chunk
            .store(config.credit_chunk, Ordering::Relaxed);
        self.configured.store(true, Ordering::Release);
    }

    fn ensure_configured(&self) {
        if self.configured.load(Ordering::Acquire) {
            return;
        }
        let mut slot = self.config.lock().unwrap_or_else(PoisonError::into_inner);
        if slot.is_some() {
            return;
        }
        let config = MemoryConfig::from_env().unwrap_or_else(|e| {
            eprintln!("[memory] warning: invalid memory configuration ({e}); using defaults");
            MemoryConfig::platform_default()
        });
        self.install(&config);
        *slot = Some(config);
    }

    /// The active config.
    pub fn config(&self) -> MemoryConfig {
        self.ensure_configured();
        self.config
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .expect("configured")
    }

    fn credit_chunk(&self) -> usize {
        self.ensure_configured();
        self.credit_chunk.load(Ordering::Relaxed)
    }

    /// Register an isolate.  The returned account is single-threaded (`!Sync`).
    pub fn register_isolate(&'static self, name: impl Into<Arc<str>>) -> IsolateAccount {
        let id = self.next_isolate_id.fetch_add(1, Ordering::Relaxed);
        let control = Arc::new(IsolateControl::new(id, name.into()));
        {
            let mut reg = self.registry.lock().unwrap_or_else(PoisonError::into_inner);
            reg.retain(|w| w.strong_count() > 0);
            reg.push(Arc::downgrade(&control));
        }
        self.isolates_registered.fetch_add(1, Ordering::Relaxed);
        IsolateAccount {
            governor: self,
            control,
            used: Cell::new(0),
            published: Cell::new(0),
            publish_step: Cell::new(self.credit_chunk()),
        }
    }

    fn unregister(&self, control: &Arc<IsolateControl>) {
        let mut reg = self.registry.lock().unwrap_or_else(PoisonError::into_inner);
        reg.retain(|w| w.strong_count() > 0 && !std::ptr::eq(w.as_ptr(), Arc::as_ptr(control)));
    }

    /// Reserve `bytes` of `class` for an allocation with an independent
    /// lifetime.  Phase 1 is observe-only: the reservation always succeeds
    /// and an over-limit reservation is recorded as a would-reject event.
    pub fn reserve_shared(
        &'static self,
        class: MemoryClass,
        bytes: usize,
    ) -> Result<MemoryCharge, MemoryLimitExceeded> {
        self.add(class, bytes);
        self.evaluate(None, bytes > 0);
        Ok(MemoryCharge {
            governor: self,
            class,
            bytes,
        })
    }

    fn add(&self, class: MemoryClass, bytes: usize) {
        if bytes > 0 {
            self.class_bytes[class.index()].fetch_add(bytes, Ordering::Relaxed);
        }
    }

    fn sub(&self, class: MemoryClass, bytes: usize) {
        if bytes == 0 {
            return;
        }
        let prev = self.class_bytes[class.index()].fetch_sub(bytes, Ordering::Relaxed);
        if prev < bytes {
            // Restore the counter before reporting, so release builds keep
            // a sane (zero) value rather than a wrapped one.  Not atomic with
            // the `fetch_sub`: a concurrent `add` in between can still lose
            // bytes.  Underflow is a bug, and this repair is best-effort.
            self.class_bytes[class.index()].fetch_add(bytes - prev, Ordering::Relaxed);
            debug_assert!(
                false,
                "{} counter underflow: released {bytes} with {prev} committed",
                class.name()
            );
        }
    }

    /// Current bytes in `class`.
    pub fn class_bytes(&self, class: MemoryClass) -> usize {
        self.class_bytes[class.index()].load(Ordering::Relaxed)
    }

    /// Total committed bytes (Phase 1: equal to used bytes).
    pub fn committed_bytes(&self) -> usize {
        self.class_bytes.iter().fold(0usize, |acc, c| {
            acc.saturating_add(c.load(Ordering::Relaxed))
        })
    }

    /// Current pressure level.
    pub fn pressure(&self) -> PressureLevel {
        PressureLevel::from_u8(self.pressure.load(Ordering::Acquire))
    }

    /// Recompute pressure; request collection at `Yellow`/`Red`.
    fn evaluate(&self, requester: Option<&IsolateControl>, grew: bool) {
        self.ensure_configured();
        let committed = self.committed_bytes();
        self.peak_committed.fetch_max(committed, Ordering::Relaxed);
        let soft = self.soft_limit.load(Ordering::Relaxed);
        let hard = self.hard_limit.load(Ordering::Relaxed);
        let level = PressureLevel::classify(committed, soft, hard);
        let prev = PressureLevel::from_u8(self.pressure.swap(level as u8, Ordering::AcqRel));
        if prev != level {
            self.pressure_transitions.fetch_add(1, Ordering::Relaxed);
            tracing::info!(
                target: "memory",
                from = %prev,
                to = %level,
                committed,
                soft_limit = soft,
                hard_limit = hard,
                "memory pressure transition"
            );
        }
        if level == PressureLevel::Red && grew {
            self.over_limit_events.fetch_add(1, Ordering::Relaxed);
            self.over_limit_bytes
                .fetch_add((committed - hard) as u64, Ordering::Relaxed);
            tracing::debug!(
                target: "memory",
                committed,
                hard_limit = hard,
                isolate = requester.map(|c| c.id),
                "governed allocation over the hard limit (observe-only; not rejected)"
            );
        }
        if level == PressureLevel::Green {
            return;
        }
        if let Some(req) = requester
            && req.governor_request(level)
        {
            self.collection_requests.fetch_add(1, Ordering::Relaxed);
        }
        if grew && let Some(largest) = self.largest_isolate() {
            let same = requester.is_some_and(|r| r.id == largest.id);
            if !same && largest.governor_request(level) {
                self.collection_requests.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    fn largest_isolate(&self) -> Option<Arc<IsolateControl>> {
        let reg = self.registry.lock().unwrap_or_else(PoisonError::into_inner);
        reg.iter()
            .filter_map(Weak::upgrade)
            .max_by_key(|c| c.used_bytes())
    }

    /// Point-in-time view of every counter.
    pub fn snapshot(&self) -> MemorySnapshot {
        let config = self.config();
        let mut class_bytes = [0usize; CLASS_COUNT];
        for (slot, c) in class_bytes.iter_mut().zip(self.class_bytes.iter()) {
            *slot = c.load(Ordering::Relaxed);
        }
        let committed = class_bytes
            .iter()
            .fold(0usize, |acc, &b| acc.saturating_add(b));
        let mut isolates: Vec<IsolateSnapshot> = {
            let reg = self.registry.lock().unwrap_or_else(PoisonError::into_inner);
            reg.iter()
                .filter_map(Weak::upgrade)
                .map(|c| c.snapshot())
                .collect()
        };
        isolates.sort_by_key(|i| i.id);
        MemorySnapshot {
            soft_limit: config.soft_limit,
            hard_limit: config.hard_limit,
            critical_reserve: config.critical_reserve,
            credit_chunk: config.credit_chunk,
            pressure: self.pressure(),
            committed_bytes: committed,
            used_bytes: committed,
            free_credit_bytes: 0,
            peak_committed_bytes: self.peak_committed.load(Ordering::Relaxed),
            class_bytes,
            isolates,
            isolates_registered: self.isolates_registered.load(Ordering::Relaxed),
            pressure_transitions: self.pressure_transitions.load(Ordering::Relaxed),
            collection_requests: self.collection_requests.load(Ordering::Relaxed),
            over_limit_events: self.over_limit_events.load(Ordering::Relaxed),
            over_limit_bytes: self.over_limit_bytes.load(Ordering::Relaxed),
        }
    }
}

impl Default for ProcessMemoryGovernor {
    fn default() -> Self {
        Self::new()
    }
}

static GOVERNOR: ProcessMemoryGovernor = ProcessMemoryGovernor::new();

/// The process governor shared by every runtime in the process.
pub fn governor() -> &'static ProcessMemoryGovernor {
    &GOVERNOR
}

// ── Snapshots ────────────────────────────────────────────────────────────────

/// Point-in-time per-isolate counters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsolateSnapshot {
    pub id: IsolateId,
    pub name: Arc<str>,
    pub used_bytes: usize,
    pub peak_used_bytes: usize,
    pub collections: u64,
    pub collection_requests: u64,
    pub zero_yield_collections: u64,
    pub last_bytes_before: usize,
    pub last_bytes_after: usize,
    pub last_duration: Duration,
}

/// Point-in-time process counters.  Fields are individually atomic
/// observations; the snapshot as a whole is not a consistent cut.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemorySnapshot {
    pub soft_limit: usize,
    pub hard_limit: usize,
    pub critical_reserve: usize,
    pub credit_chunk: usize,
    pub pressure: PressureLevel,
    pub committed_bytes: usize,
    pub used_bytes: usize,
    /// Always 0 until chunked credit (Phase 2).
    pub free_credit_bytes: usize,
    pub peak_committed_bytes: usize,
    class_bytes: [usize; CLASS_COUNT],
    /// Live registered isolates.
    pub isolates: Vec<IsolateSnapshot>,
    /// Isolates ever registered.
    pub isolates_registered: u64,
    pub pressure_transitions: u64,
    pub collection_requests: u64,
    /// Growth events that ended above the hard limit.  Strict mode would
    /// have rejected each one.
    pub over_limit_events: u64,
    /// Sum of the overage at each over-limit event.
    pub over_limit_bytes: u64,
}

impl MemorySnapshot {
    /// Current bytes in `class`.
    pub fn class_bytes(&self, class: MemoryClass) -> usize {
        self.class_bytes[class.index()]
    }
}

impl fmt::Display for MemorySnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Managed memory (observe-only):")?;
        writeln!(
            f,
            "  Limits:                soft {} / hard {} bytes (reserve {})",
            self.soft_limit, self.hard_limit, self.critical_reserve
        )?;
        writeln!(f, "  Pressure:              {}", self.pressure)?;
        writeln!(
            f,
            "  Committed:             {} bytes (peak {})",
            self.committed_bytes, self.peak_committed_bytes
        )?;
        for class in MemoryClass::ALL {
            let bytes = self.class_bytes(class);
            if bytes > 0 {
                writeln!(f, "    {:<20} {bytes} bytes", format!("{}:", class.name()))?;
            }
        }
        writeln!(
            f,
            "  Isolates:              {} live ({} registered)",
            self.isolates.len(),
            self.isolates_registered
        )?;
        for iso in &self.isolates {
            writeln!(
                f,
                "    #{} {}: {} bytes (peak {}), {} collections, {} requested",
                iso.id,
                iso.name,
                iso.used_bytes,
                iso.peak_used_bytes,
                iso.collections,
                iso.collection_requests
            )?;
        }
        writeln!(f, "  Pressure transitions:  {}", self.pressure_transitions)?;
        writeln!(f, "  Collection requests:   {}", self.collection_requests)?;
        write!(
            f,
            "  Over hard limit:       {} events ({} bytes over)",
            self.over_limit_events, self.over_limit_bytes
        )
    }
}

// ── Current-thread isolate ───────────────────────────────────────────────────

thread_local! {
    static CURRENT: OnceCell<IsolateAccount> = const { OnceCell::new() };
}

fn default_isolate_name() -> Arc<str> {
    let thread = std::thread::current();
    match thread.name() {
        Some(name) => Arc::from(name),
        None => Arc::from(format!("{:?}", thread.id())),
    }
}

/// Register the calling thread as an isolate named `name` (or rename it if
/// it is already registered).  Returns its control handle.
pub fn register_current_isolate(name: &str) -> Option<Arc<IsolateControl>> {
    CURRENT
        .try_with(|cell| {
            if let Some(account) = cell.get() {
                account.control.set_name(Arc::from(name));
            } else {
                let _ = cell.set(governor().register_isolate(name));
            }
            cell.get().map(|a| a.control.clone())
        })
        .ok()
        .flatten()
}

/// Run `f` against the calling thread's account, registering it under the
/// thread's name on first use.  `None` during thread-local teardown.
#[inline]
pub fn with_current_account<R>(f: impl FnOnce(&IsolateAccount) -> R) -> Option<R> {
    CURRENT
        .try_with(
            |cell| f(cell.get_or_init(|| governor().register_isolate(default_isolate_name()))),
        )
        .ok()
}

/// The calling thread's control handle.
pub fn current_control() -> Option<Arc<IsolateControl>> {
    with_current_account(|a| a.control.clone())
}

/// Flush the calling thread's account and return a process snapshot.
pub fn snapshot() -> MemorySnapshot {
    let _ = CURRENT.try_with(|cell| {
        if let Some(account) = cell.get() {
            account.publish();
        }
    });
    governor().snapshot()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaked(soft: usize, hard: usize, chunk: usize) -> &'static ProcessMemoryGovernor {
        let mut config = MemoryConfig::with_limits(soft, hard).unwrap();
        config.credit_chunk = chunk;
        Box::leak(Box::new(
            ProcessMemoryGovernor::with_config(config).unwrap(),
        ))
    }

    fn report(before: usize, after: usize) -> CollectionReport {
        CollectionReport {
            bytes_before: before,
            bytes_after: after,
            bytes_returned: before - after,
            duration: Duration::from_micros(1),
        }
    }

    #[test]
    fn config_rejects_invalid_states() {
        assert_eq!(
            MemoryConfig::with_limits(0, 0),
            Err(MemoryConfigError::ZeroHardLimit)
        );
        assert!(matches!(
            MemoryConfig::with_limits(10 * MIB, 5 * MIB),
            Err(MemoryConfigError::SoftAboveHard { .. })
        ));
        let mut c = MemoryConfig::with_limits(MIB, 2 * MIB).unwrap();
        c.critical_reserve = 3 * MIB;
        assert!(matches!(
            c.validate(),
            Err(MemoryConfigError::ReserveAboveHard { .. })
        ));
        let mut c = MemoryConfig::with_limits(MIB, 2 * MIB).unwrap();
        c.credit_chunk = 0;
        assert_eq!(c.validate(), Err(MemoryConfigError::ZeroCreditChunk));
        let mut c = MemoryConfig::with_limits(MIB, 2 * MIB).unwrap();
        c.queue_limit = 3 * MIB;
        assert!(matches!(
            c.validate(),
            Err(MemoryConfigError::QueueAboveBudget { .. })
        ));
    }

    #[test]
    fn config_defaults() {
        let c = MemoryConfig::with_limits(300 * MIB, 400 * MIB).unwrap();
        assert_eq!(c.critical_reserve, 4 * MIB);
        assert_eq!(c.credit_chunk, DEFAULT_CREDIT_CHUNK);
        assert_eq!(c.queue_limit, DEFAULT_QUEUE_LIMIT);
        let c = MemoryConfig::with_limits(1, 2 * MIB).unwrap();
        assert_eq!(
            c.critical_reserve,
            2 * MIB,
            "reserve is capped at the hard limit"
        );
        assert_eq!(c.queue_limit, 2 * MIB);
        let big = MemoryConfig::with_limits(1, 1 << 40).unwrap();
        assert_eq!(big.critical_reserve, MAX_DEFAULT_CRITICAL_RESERVE);
        let c = MemoryConfig::from_optional_limits(None, Some(400 * MIB)).unwrap();
        assert_eq!(c.soft_limit, 300 * MIB);
        assert_eq!(
            default_soft_limit(1),
            1,
            "tiny hard limit keeps a nonzero soft limit"
        );
        assert_eq!(default_soft_limit(3), 3);
        assert_eq!(default_soft_limit(4), 3);
        let c = MemoryConfig::from_optional_limits(Some(usize::MAX / 2), None).unwrap();
        assert_eq!(
            c.soft_limit, c.hard_limit,
            "soft-only raises the hard limit"
        );
    }

    #[test]
    fn env_new_names_win_and_old_names_warn() {
        let vars = [
            (SOFT_LIMIT_ENV, "100"),
            (DEPRECATED_SOFT_LIMIT_ENV, "50"),
            (DEPRECATED_HARD_LIMIT_ENV, "200"),
            (CREDIT_ENV, "128"),
        ];
        let lookup = |k: &str| {
            vars.iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| v.to_string())
        };
        let mut warnings = Vec::new();
        let c = MemoryConfig::from_lookup(lookup, |w| warnings.push(w)).unwrap();
        assert_eq!(c.soft_limit, 100 * MIB);
        assert_eq!(c.hard_limit, 200 * MIB);
        assert_eq!(c.credit_chunk, 128 * KIB);
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(warnings[0].contains("ignored"));
        assert!(warnings[1].contains(DEPRECATED_HARD_LIMIT_ENV));
    }

    #[test]
    fn env_rejects_bad_values() {
        let err = MemoryConfig::from_lookup(
            |k| (k == HARD_LIMIT_ENV).then(|| "lots".to_string()),
            |_| {},
        )
        .unwrap_err();
        assert!(matches!(err, MemoryConfigError::InvalidNumber { .. }));
        let err = MemoryConfig::from_lookup(
            |k| match k {
                SOFT_LIMIT_ENV => Some("20".into()),
                HARD_LIMIT_ENV => Some("10".into()),
                _ => None,
            },
            |_| {},
        )
        .unwrap_err();
        assert!(matches!(err, MemoryConfigError::SoftAboveHard { .. }));
    }

    #[test]
    fn charge_publishes_once_per_chunk() {
        let g = leaked(10 * MIB, 20 * MIB, 4 * KIB);
        let acc = g.register_isolate("a");
        acc.charge(KIB);
        assert_eq!(
            g.class_bytes(MemoryClass::GcHeap),
            0,
            "below one chunk stays local"
        );
        acc.charge(3 * KIB);
        assert_eq!(g.class_bytes(MemoryClass::GcHeap), 4 * KIB);
        assert_eq!(acc.control().used_bytes(), 4 * KIB);
        acc.release(4 * KIB);
        acc.publish();
        assert_eq!(g.class_bytes(MemoryClass::GcHeap), 0);
    }

    #[test]
    fn sweep_returns_exact_bytes() {
        let g = leaked(10 * MIB, 20 * MIB, KIB);
        let acc = g.register_isolate("a");
        for _ in 0..100 {
            acc.charge(48);
        }
        acc.release(48 * 60);
        acc.record_collection(report(4800, 4800 - 48 * 60));
        assert_eq!(g.class_bytes(MemoryClass::GcHeap), 48 * 40);
        assert_eq!(acc.control().collection_epoch(), 1);
    }

    #[test]
    fn shutdown_returns_all_bytes_and_unregisters() {
        let g = leaked(10 * MIB, 20 * MIB, KIB);
        let acc = g.register_isolate("a");
        acc.charge(10 * KIB);
        assert_eq!(g.snapshot().isolates.len(), 1);
        drop(acc);
        let snap = g.snapshot();
        assert_eq!(snap.committed_bytes, 0);
        assert!(snap.isolates.is_empty());
        assert_eq!(snap.isolates_registered, 1);
    }

    #[test]
    fn shared_charge_returns_once() {
        let g = leaked(10 * MIB, 20 * MIB, KIB);
        let charge = Arc::new(g.reserve_shared(MemoryClass::SharedValue, 1000).unwrap());
        let clone = charge.clone();
        assert_eq!(g.class_bytes(MemoryClass::SharedValue), 1000);
        drop(charge);
        assert_eq!(g.class_bytes(MemoryClass::SharedValue), 1000);
        drop(clone);
        assert_eq!(g.class_bytes(MemoryClass::SharedValue), 0);
    }

    #[test]
    fn pressure_transitions_and_requests() {
        let g = leaked(4 * MIB, 8 * MIB, 64 * KIB);
        let acc = g.register_isolate("a");
        acc.charge(MIB);
        assert_eq!(g.pressure(), PressureLevel::Green);
        assert!(!acc.control().collection_requested());

        acc.charge(3 * MIB);
        assert_eq!(g.pressure(), PressureLevel::Yellow);
        assert!(
            acc.control().take_collection_request(),
            "Yellow requests collection"
        );

        // Rate limit: no second request in the same collection epoch.
        acc.charge(64 * KIB);
        assert!(!acc.control().collection_requested());

        // After a zero-yield collection the next request waits for growth.
        acc.record_collection(CollectionReport {
            bytes_before: acc.used_bytes(),
            bytes_after: acc.used_bytes(),
            bytes_returned: 0,
            duration: Duration::ZERO,
        });
        acc.charge(64 * KIB);
        assert!(
            !acc.control().collection_requested(),
            "Yellow honours the target"
        );

        // Red overrides the target.
        acc.charge(5 * MIB);
        assert_eq!(g.pressure(), PressureLevel::Red);
        assert!(acc.control().take_collection_request());
        let snap = g.snapshot();
        assert!(snap.over_limit_events >= 1);
        assert!(snap.pressure_transitions >= 2);

        acc.release(acc.used_bytes());
        acc.publish();
        assert_eq!(g.pressure(), PressureLevel::Green);
    }

    #[test]
    fn yellow_requests_largest_heap() {
        let g = leaked(4 * MIB, 8 * MIB, 64 * KIB);
        let big = g.register_isolate("big");
        let small = g.register_isolate("small");
        big.charge(3 * MIB);
        assert!(!big.control().collection_requested());
        small.charge(MIB);
        assert_eq!(g.pressure(), PressureLevel::Yellow);
        assert!(small.control().take_collection_request());
        assert!(
            big.control().take_collection_request(),
            "largest heap asked too"
        );
    }

    #[test]
    fn isolates_share_one_budget_across_threads() {
        let g = leaked(64 * MIB, 128 * MIB, 4 * KIB);
        let threads: Vec<_> = (0..8)
            .map(|i| {
                std::thread::spawn(move || {
                    let acc = g.register_isolate(format!("iso-{i}"));
                    for _ in 0..1000 {
                        acc.charge(1000);
                    }
                    acc.publish();
                    let peak = g.committed_bytes();
                    acc.release(acc.used_bytes());
                    acc.publish();
                    peak
                })
            })
            .collect();
        for t in threads {
            assert!(t.join().unwrap() >= 1_000_000);
        }
        assert_eq!(g.committed_bytes(), 0, "no counter leaks");
        assert!(g.snapshot().peak_committed_bytes >= 1_000_000);
    }
}
