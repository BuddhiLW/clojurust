//! Execution mode and tier state.
//!
//! [`ExecutionMode`] is chosen once, when a runtime is built, and never
//! changes: it selects which function-call path a runtime uses.  Before the
//! merge each mode was expressed by storing a different `fn` pointer in
//! `GlobalEnv`; the pointers existed only to let `cljrs-interp` reach
//! `cljrs-eval` without a dependency cycle.  Now that both live in this
//! package the mode is data and the dispatch is a direct call.
//!
//! [`TierState`] is the *current* state of that mode, and it does change: a
//! tiered runtime tree-walks its own bootstrap (nothing can be lowered before
//! `clojure.core` exists) and is promoted to [`TierState::Ir`] or
//! [`TierState::Jit`] once the bootstrap finishes.  It replaces the old
//! `GlobalEnv::compiler_ready` flag, which said only "not tree-walk" and could
//! not distinguish the IR interpreter from native JIT dispatch.

/// How a runtime executes Clojure function calls.
///
/// Selected with [`crate::RuntimeBuilder::execution_mode`] and fixed for the
/// life of the runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExecutionMode {
    /// Tree-walking interpreter only.  No IR is lowered, cached, or executed.
    ///
    /// The cheapest mode to construct and the one to use for short-lived
    /// environments (tests, one-shot evaluation, the AOT test harness) where
    /// populating the IR cache would be pure overhead.
    TreeWalk,

    /// Tree walk, tier-1 IR interpreter, and native JIT promotion.
    ///
    /// The default, and what the `cljrs` CLI uses.  Native dispatch is only
    /// reached once a JIT backend is attached to the runtime
    /// (`cljrs_compiler::jit::install`); without one this behaves like
    /// [`ExecutionMode::TieredNoJit`].
    #[default]
    Tiered,

    /// Tree walk and tier-1 IR interpreter, with native JIT promotion
    /// suppressed even when a JIT backend is linked in.
    TieredNoJit,

    /// Tree walk inside a no-GC transaction arena, with a call-depth cap.
    ///
    /// Used by `cljrs-tx`, where interpreted recursion runs on the host's
    /// Rust stack and must be bounded.  Install the cap for the dynamic
    /// extent of one transaction with [`crate::env::depth::DepthGuard`].
    NoGcTransaction,
}

impl ExecutionMode {
    /// The tier this mode is promoted to once bootstrap finishes.
    pub fn target_tier(self) -> TierState {
        match self {
            ExecutionMode::TreeWalk | ExecutionMode::NoGcTransaction => TierState::TreeWalk,
            ExecutionMode::Tiered => TierState::Jit,
            ExecutionMode::TieredNoJit => TierState::Ir,
        }
    }

    /// True when function calls go through the IR-aware dispatch path
    /// (`crate::tiered::apply::call_cljrs_fn`) rather than straight to the
    /// tree walker.
    pub fn is_tiered(self) -> bool {
        matches!(self, ExecutionMode::Tiered | ExecutionMode::TieredNoJit)
    }
}

/// Which execution tiers are live right now.
///
/// Monotonic within a runtime's life: it starts at [`TierState::TreeWalk`]
/// while `clojure.core` bootstraps and is raised once to
/// [`ExecutionMode::target_tier`] when the runtime is ready.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum TierState {
    /// Only the tree walker runs.  No lowering is attempted.
    TreeWalk = 0,
    /// Tree walk plus IR lowering and the tier-1 IR interpreter.
    Ir = 1,
    /// Tree walk, IR, and native code published by a JIT backend.
    Jit = 2,
}

impl TierState {
    /// Decode from the `AtomicU8` representation held by `GlobalEnv`.
    /// Unknown values decode as [`TierState::TreeWalk`].
    pub(crate) fn from_u8(raw: u8) -> Self {
        match raw {
            1 => TierState::Ir,
            2 => TierState::Jit,
            _ => TierState::TreeWalk,
        }
    }

    /// True when IR may be lowered, cached, and interpreted.
    pub fn ir_enabled(self) -> bool {
        self >= TierState::Ir
    }

    /// True when dispatch may jump to native code published by a JIT backend.
    pub fn jit_enabled(self) -> bool {
        self == TierState::Jit
    }
}
