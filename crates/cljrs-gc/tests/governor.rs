//! Process memory governor integration tests (GC mode).
//!
//! Each integration-test binary is its own process, so these tests own the
//! process governor.  They serialize on `LOCK` because they share it.

#![cfg(not(feature = "no-gc"))]

use std::sync::{Arc, Barrier, Mutex};

use cljrs_gc::governor::{self, MemoryClass, MemoryConfig, PressureLevel};
use cljrs_gc::{GcConfig, HEAP};

const MIB: usize = 1024 * 1024;

static LOCK: Mutex<()> = Mutex::new(());

/// Allocate `mib` MiB of heap-accounted strings on this thread's heap.
fn allocate_mib(mib: usize) -> Vec<cljrs_gc::GcPtr<String>> {
    (0..mib * 16)
        .map(|_| HEAP.alloc(String::with_capacity(64 * 1024)))
        .collect()
}

/// Phase 0 regression: every isolate heap applies the configured soft limit
/// on its own, so N isolates can hold N times that limit without any heap
/// requesting collection.  The process governor sees the combined total.
#[test]
fn per_isolate_limits_multiply_but_governor_sees_the_total() {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let soft = 3 * MIB;
    governor::governor()
        .configure(MemoryConfig::with_limits(soft, 16 * MIB).unwrap())
        .unwrap();

    let barrier = Arc::new(Barrier::new(3));
    let workers: Vec<_> = (0..2)
        .map(|i| {
            let barrier = barrier.clone();
            std::thread::Builder::new()
                .name(format!("multiply-{i}"))
                .spawn(move || {
                    cljrs_gc::register_current_isolate(&format!("multiply-{i}"));
                    HEAP.set_config(Arc::new(GcConfig::with_limits(soft, 16 * MIB)));
                    let _live = allocate_mib(2);
                    // Flush this isolate's account so the totals are exact.
                    let _ = governor::snapshot();
                    let in_use = HEAP.memory_in_use();
                    barrier.wait(); // both isolates at peak
                    barrier.wait(); // main thread has inspected the governor
                    in_use
                })
                .unwrap()
        })
        .collect();

    barrier.wait();
    let snap = governor::governor().snapshot();
    barrier.wait();
    let per_heap: Vec<usize> = workers.into_iter().map(|w| w.join().unwrap()).collect();

    for in_use in &per_heap {
        assert!(
            *in_use >= 2 * MIB && *in_use < soft,
            "each heap stays under its own soft limit: {in_use}"
        );
    }
    let total: usize = per_heap.iter().sum();
    assert!(
        total > soft,
        "together the heaps exceed the configured limit"
    );

    let heap_bytes = snap.class_bytes(MemoryClass::GcHeap);
    assert!(
        heap_bytes >= total,
        "governor counts both heaps: {heap_bytes} < {total}"
    );
    assert_eq!(snap.pressure, PressureLevel::Yellow);
    assert!(snap.collection_requests >= 1, "Yellow requests collection");
    let names: Vec<&str> = snap.isolates.iter().map(|i| &*i.name).collect();
    assert!(
        names.contains(&"multiply-0") && names.contains(&"multiply-1"),
        "{names:?}"
    );
    let requested: u64 = snap
        .isolates
        .iter()
        .filter(|i| i.name.starts_with("multiply-"))
        .map(|i| i.collection_requests)
        .sum();
    assert!(requested >= 1);

    // Thread exit unregisters both isolates and returns their charge.
    let after = governor::governor().snapshot();
    assert!(
        after
            .isolates
            .iter()
            .all(|i| !i.name.starts_with("multiply-")),
        "{:?}",
        after.isolates
    );
}

/// Sweep returns exactly the bytes recorded in each freed header.
#[test]
fn account_tracks_heap_exactly_through_collection() {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    governor::governor()
        .configure(MemoryConfig::with_limits(512 * MIB, 1024 * MIB).unwrap())
        .unwrap();
    std::thread::spawn(|| {
        let account_bytes = || governor::with_current_account(|a| a.used_bytes()).unwrap();
        drop(allocate_mib(1));
        assert_eq!(account_bytes(), HEAP.memory_in_use());
        let epoch = governor::current_control().unwrap().collection_epoch();
        HEAP.collect(|_| {});
        HEAP.collect(|_| {});
        assert_eq!(account_bytes(), HEAP.memory_in_use());
        assert_eq!(account_bytes(), 0);
        let control = governor::current_control().unwrap();
        assert_eq!(control.collection_epoch(), epoch + 2);
        assert_eq!(control.used_bytes(), 0, "collection publishes");
    })
    .join()
    .unwrap();
}

/// A collection request from the governor reaches the target isolate's
/// safepoint check, from another thread.
#[test]
fn governor_request_crosses_threads() {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (tx, rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
    let worker = std::thread::spawn(move || {
        let control = cljrs_gc::register_current_isolate("remote-target").unwrap();
        assert!(!cljrs_gc::gc_requested());
        tx.send(control).unwrap();
        done_rx.recv().unwrap();
        let seen = cljrs_gc::gc_requested();
        let taken = cljrs_gc::take_gc_request();
        (seen, taken, cljrs_gc::gc_requested())
    });
    let control = rx.recv().unwrap();
    control.request_collection();
    done_tx.send(()).unwrap();
    assert_eq!(worker.join().unwrap(), (true, true, false));
}

/// Without a fixed per-heap trigger, a heap requests its own collection when
/// it passes the governor's dynamic target, while the process is `Green`.
/// After the collection, the account keeps at most the retained credit.
#[test]
fn heap_without_fixed_trigger_collects_at_dynamic_target() {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    governor::governor()
        .configure(MemoryConfig::with_limits(512 * MIB, 1024 * MIB).unwrap())
        .unwrap();
    std::thread::spawn(|| {
        let control = cljrs_gc::register_current_isolate("dynamic-target").unwrap();
        let target = control.collection_target();
        assert_eq!(target, governor::governor().min_headroom());
        let below = allocate_mib(target / MIB - 1);
        assert!(!cljrs_gc::gc_requested(), "below the target");
        let above = allocate_mib(2);
        assert!(cljrs_gc::take_gc_request(), "past the target");
        assert_eq!(governor::governor().pressure(), PressureLevel::Green);

        drop((below, above));
        HEAP.collect(|_| {});
        HEAP.collect(|_| {});
        let (used, free, committed) = governor::with_current_account(|a| {
            (a.used_bytes(), a.free_credit_bytes(), a.committed_bytes())
        })
        .unwrap();
        assert_eq!(used, HEAP.memory_in_use());
        let config = governor::governor().config();
        assert!(free <= config.retained_credit_chunks * config.credit_chunk);
        assert_eq!(committed, used + free);
    })
    .join()
    .unwrap();
}

/// Eight isolates share one budget with no fixed partition: one active
/// isolate commits several times an eighth of the budget while seven idle
/// isolates hold at most their retained credit.
#[test]
fn active_isolate_borrows_idle_capacity() {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let hard = 64 * MIB;
    governor::governor()
        .configure(MemoryConfig::with_limits(48 * MIB, hard).unwrap())
        .unwrap();
    let barrier = Arc::new(Barrier::new(8));
    let idle: Vec<_> = (0..7)
        .map(|i| {
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                cljrs_gc::register_current_isolate(&format!("idle-{i}"));
                let _small: Vec<_> = (0..100).map(|n| HEAP.alloc(n as i64)).collect();
                barrier.wait(); // allocated
                barrier.wait(); // the active isolate has peaked
                governor::with_current_account(|a| a.committed_bytes()).unwrap()
            })
        })
        .collect();
    barrier.wait();
    let active = std::thread::spawn(|| {
        cljrs_gc::register_current_isolate("active");
        let live = allocate_mib(24);
        let committed = governor::with_current_account(|a| a.committed_bytes()).unwrap();
        drop(live);
        committed
    })
    .join()
    .unwrap();
    barrier.wait();
    let chunk = governor::governor().config().credit_chunk;
    for committed in idle.into_iter().map(|t| t.join().unwrap()) {
        assert!(committed <= chunk, "idle isolate holds {committed} bytes");
    }
    assert!(
        active > 3 * (hard / 8),
        "active isolate committed {active} bytes"
    );
}
