//! Cooperative execution-credit metering shared by every evaluation tier.
//!
//! Meter installation is thread-local; compiled async state machines explicitly
//! capture and reinstall the active meter stack when they are spawned. Nested meters
//! are charged together so inner evaluations cannot escape an outer budget.
//! Native code reports exhaustion through per-scope sticky thread-local flags,
//! allowing the signal to survive callback/JIT bridge boundaries without
//! contaminating a healthy enclosing or subsequent scope.
//!
//! The same checkpoints also observe *interrupt flags*: an embedder (e.g. the
//! nREPL server) installs an `Arc<AtomicBool>` with [`InterruptGuard`] for the
//! extent of an evaluation, and another thread sets it to stop that
//! evaluation. A set flag makes every subsequent charge fail exactly like an
//! exhausted budget, so the evaluation unwinds through the same uncatchable
//! [`EvalError::GasExhausted`](crate::env::error::EvalError::GasExhausted)
//! path in every tier (tree-walker, IR interpreter, JIT). Use
//! [`interrupt_requested`] to tell an interrupt from real exhaustion. When no
//! interrupt is pending the cost is one relaxed atomic load per charge.

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// A shareable, monotonically-decreasing execution-credit budget.
#[derive(Debug)]
pub struct GasMeter {
    remaining: AtomicU64,
}

impl GasMeter {
    pub fn new(credits: u64) -> Arc<Self> {
        Arc::new(Self {
            remaining: AtomicU64::new(credits),
        })
    }

    pub fn remaining(&self) -> u64 {
        self.remaining.load(Ordering::Relaxed)
    }

    /// Consume `cost` credits, returning false without partially charging when
    /// the budget cannot cover the whole checkpoint.
    pub fn charge(&self, cost: u64) -> bool {
        let mut remaining = self.remaining.load(Ordering::Relaxed);
        loop {
            let Some(left) = remaining.checked_sub(cost) else {
                return false;
            };
            match self.remaining.compare_exchange_weak(
                remaining,
                left,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return true,
                Err(current) => remaining = current,
            }
        }
    }
}

thread_local! {
    static ACTIVE: RefCell<Vec<Arc<GasMeter>>> = const { RefCell::new(Vec::new()) };
    static EXHAUSTED: RefCell<Vec<bool>> = const { RefCell::new(Vec::new()) };
    static INTERRUPTS: RefCell<Vec<Arc<AtomicBool>>> = const { RefCell::new(Vec::new()) };
}

/// Installs an interrupt flag for the dynamic extent of an evaluation on
/// this thread. While installed, setting the flag (from any thread) makes the
/// next execution-credit checkpoint fail, unwinding the evaluation as
/// `GasExhausted`; [`interrupt_requested`] then reports true.
#[must_use = "dropping InterruptGuard immediately uninstalls the interrupt flag"]
pub struct InterruptGuard;

impl InterruptGuard {
    pub fn install(flag: Arc<AtomicBool>) -> Self {
        INTERRUPTS.with(|flags| flags.borrow_mut().push(flag));
        Self
    }
}

impl Drop for InterruptGuard {
    fn drop(&mut self) {
        INTERRUPTS.with(|flags| {
            flags.borrow_mut().pop();
        });
    }
}

/// True when an interrupt flag installed on this thread has been set.
pub fn interrupt_requested() -> bool {
    INTERRUPTS.with(|flags| {
        flags
            .borrow()
            .iter()
            .any(|flag| flag.load(Ordering::Relaxed))
    })
}

/// Installs a meter for the dynamic extent of an evaluation.
#[must_use = "dropping GasGuard immediately uninstalls the active gas meter"]
pub struct GasGuard;

impl GasGuard {
    pub fn install(meter: Arc<GasMeter>) -> Self {
        ACTIVE.with(|active| active.borrow_mut().push(meter));
        EXHAUSTED.with(|exhausted| exhausted.borrow_mut().push(false));
        Self
    }
}

impl Drop for GasGuard {
    fn drop(&mut self) {
        ACTIVE.with(|active| {
            let mut active = active.borrow_mut();
            active.pop();
        });
        EXHAUSTED.with(|exhausted| {
            exhausted.borrow_mut().pop();
        });
    }
}

/// Charge the active evaluation, or succeed at no cost when unmetered.
pub fn charge(cost: u64) -> bool {
    if interrupt_requested() || is_exhausted() {
        return false;
    }
    let charged = ACTIVE.with(|active| {
        let active = active.borrow();
        if active.is_empty() {
            return true;
        }
        if active.iter().any(|meter| meter.remaining() < cost) {
            return false;
        }
        active.iter().all(|meter| meter.charge(cost))
    });
    if !charged {
        ACTIVE.with(|active| {
            let active = active.borrow();
            EXHAUSTED.with(|exhausted| {
                for (index, meter) in active.iter().enumerate() {
                    if meter.remaining() < cost {
                        exhausted.borrow_mut()[index] = true;
                    }
                }
            });
        });
    }
    charged
}

/// Peek at the native-tier exhaustion signal set by a failed charge.
pub fn is_exhausted() -> bool {
    interrupt_requested()
        || EXHAUSTED.with(|exhausted| exhausted.borrow().iter().any(|value| *value))
}

/// Clone the complete active meter stack for async task propagation.
pub fn active_meters() -> Vec<Arc<GasMeter>> {
    ACTIVE.with(|active| active.borrow().clone())
}

/// Install a captured meter stack in outer-to-inner order.
pub fn install_meters(meters: &[Arc<GasMeter>]) -> Vec<GasGuard> {
    meters.iter().cloned().map(GasGuard::install).collect()
}

/// Take the native-tier exhaustion signal set by a failed charge.
///
/// Prefer [`is_exhausted`] at dispatch boundaries; this remains available for
/// tests and rare code that intentionally owns the current gas scope.
pub fn take_exhausted() -> bool {
    EXHAUSTED.with(|exhausted| {
        let mut exhausted = exhausted.borrow_mut();
        let was_exhausted = exhausted.iter().any(|value| *value);
        exhausted.fill(false);
        was_exhausted
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoped_meter_charges_without_partial_consumption() {
        let meter = GasMeter::new(3);
        let _guard = GasGuard::install(meter.clone());
        assert!(charge(2));
        assert!(!charge(2));
        assert_eq!(meter.remaining(), 1);
    }

    #[test]
    fn nested_meters_charge_outer_budget() {
        let outer = GasMeter::new(3);
        let _outer_guard = GasGuard::install(outer.clone());
        let inner = GasMeter::new(2);
        let _inner_guard = GasGuard::install(inner.clone());
        assert!(charge(2));
        assert_eq!(outer.remaining(), 1);
        assert_eq!(inner.remaining(), 0);
        assert!(!charge(1));
    }

    #[test]
    fn inner_exhaustion_does_not_poison_healthy_outer_scope() {
        let outer = GasMeter::new(10);
        let _outer_guard = GasGuard::install(outer.clone());
        {
            let inner = GasMeter::new(0);
            let _inner_guard = GasGuard::install(inner);
            assert!(!charge(1));
            assert!(is_exhausted());
        }
        assert!(!is_exhausted());
        assert!(charge(1));
        assert_eq!(outer.remaining(), 9);
    }

    #[test]
    fn interrupt_flag_fails_charges_until_uninstalled() {
        let flag = Arc::new(AtomicBool::new(false));
        {
            let _guard = InterruptGuard::install(flag.clone());
            assert!(charge(1));
            flag.store(true, Ordering::Relaxed);
            assert!(!charge(1));
            assert!(is_exhausted());
            assert!(interrupt_requested());
        }
        assert!(!interrupt_requested());
        assert!(!is_exhausted());
        assert!(charge(1));
    }
}
