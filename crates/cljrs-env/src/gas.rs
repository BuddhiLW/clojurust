//! Cooperative execution-credit metering shared by every evaluation tier.

use std::cell::{Cell, RefCell};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

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
        self.remaining
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |remaining| {
                remaining.checked_sub(cost)
            })
            .is_ok()
    }
}

thread_local! {
    static ACTIVE: RefCell<Vec<Arc<GasMeter>>> = const { RefCell::new(Vec::new()) };
    static EXHAUSTED: Cell<bool> = const { Cell::new(false) };
}

/// Installs a meter for the dynamic extent of an evaluation.
pub struct GasGuard;

impl GasGuard {
    pub fn install(meter: Arc<GasMeter>) -> Self {
        EXHAUSTED.with(|exhausted| exhausted.set(false));
        ACTIVE.with(|active| active.borrow_mut().push(meter));
        Self
    }
}

impl Drop for GasGuard {
    fn drop(&mut self) {
        ACTIVE.with(|active| {
            active.borrow_mut().pop();
        });
        EXHAUSTED.with(|exhausted| exhausted.set(false));
    }
}

/// Charge the active evaluation, or succeed at no cost when unmetered.
pub fn charge(cost: u64) -> bool {
    let charged = ACTIVE.with(|active| {
        active
            .borrow()
            .last()
            .is_none_or(|meter| meter.charge(cost))
    });
    if !charged {
        EXHAUSTED.with(|exhausted| exhausted.set(true));
    }
    charged
}

/// Take the native-tier exhaustion signal set by a failed charge.
pub fn take_exhausted() -> bool {
    EXHAUSTED.with(|exhausted| exhausted.replace(false))
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
}
