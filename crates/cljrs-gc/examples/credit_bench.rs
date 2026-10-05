//! Allocation throughput and credit-refill rate for several credit chunk
//! sizes (`docs/managed-memory-governor-plan.md`, Phase 2).
//!
//! ```text
//! cargo run --release -p cljrs-gc --example credit_bench
//! ```
//!
//! Each run spawns fresh isolate threads, so every account starts with the
//! configured chunk.  Only allocation time is measured; the collections that
//! free each batch are excluded.

use std::time::{Duration, Instant};

use cljrs_gc::governor::{self, MemoryConfig};
use cljrs_gc::{GcPtr, HEAP, MarkVisitor, Trace};

const KIB: usize = 1024;
const MIB: usize = 1024 * KIB;
const BATCH: usize = 100_000;
const BATCHES: usize = 20;

/// A small object, typical of a boxed numeric or a short list cell.
#[allow(dead_code)] // the payload only gives the object its size
struct Cell([u64; 3]);

impl Trace for Cell {
    fn trace(&self, _: &mut MarkVisitor) {}
}

/// Allocate `BATCHES` batches on this thread; return the allocation time.
fn allocate() -> Duration {
    let mut elapsed = Duration::ZERO;
    let mut batch: Vec<GcPtr<Cell>> = Vec::with_capacity(BATCH);
    for _ in 0..BATCHES {
        let start = Instant::now();
        for i in 0..BATCH {
            batch.push(HEAP.alloc(Cell([i as u64; 3])));
        }
        elapsed += start.elapsed();
        batch.clear();
        HEAP.collect(|_| {});
    }
    HEAP.collect(|_| {});
    elapsed
}

fn run(threads: usize, chunk: usize) {
    let mut config = MemoryConfig::with_limits(3 * 1024 * MIB, 4 * 1024 * MIB).unwrap();
    config.credit_chunk = chunk;
    governor::governor().configure(config).unwrap();
    let before = governor::governor().snapshot();
    let stats_before = cljrs_gc::GC_STATS.snapshot();
    let wall = Instant::now();
    let per_thread: Vec<Duration> = (0..threads)
        .map(|i| {
            std::thread::Builder::new()
                .name(format!("bench-{i}"))
                .spawn(allocate)
                .unwrap()
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|t| t.join().unwrap())
        .collect();
    let wall = wall.elapsed();
    let after = governor::governor().snapshot();
    // Thread exit flushed each account's batched allocation statistics.
    let bytes = (cljrs_gc::GC_STATS.snapshot().gc_alloc_bytes - stats_before.gc_alloc_bytes) as f64;
    let allocs = (threads * BATCH * BATCHES) as f64;
    let alloc_time: Duration = per_thread.iter().sum();
    let refills = after.credit_refills - before.credit_refills;
    println!(
        "{threads:>2} threads  chunk {:>5} KiB  {:>6.1} ns/alloc  {:>9} refills  {:>8.2} refills/MiB  {:>7.1} allocs/refill  wall {:.2?}",
        chunk / KIB,
        alloc_time.as_nanos() as f64 / allocs,
        refills,
        refills as f64 / (bytes / MIB as f64),
        allocs / refills.max(1) as f64,
        wall,
    );
}

fn main() {
    // Warm up the allocator and the thread-local heap machinery.
    run(1, 64 * KIB);
    for threads in [1, 8] {
        for chunk in [4 * KIB, 16 * KIB, 64 * KIB, 256 * KIB, MIB] {
            run(threads, chunk);
        }
    }
}
