# cljrs-tx

## Purpose

Runs pure Datomic-style transaction functions in a fresh, GC-less,
tree-walker-only invocation arena.

## Status

Initial transaction-runtime implementation. IR lowering, JIT, AOT, async,
I/O, clocks, randomness, process-global mutable values, and Rust interop are
outside this crate's execution profile.

Build with the crate's `no-gc` feature (`cargo test -p cljrs-tx --features
no-gc`). The feature is not enabled by default so workspace-wide GC builds do
not accidentally unify every clojurust dependency into `no-gc` mode.

## File layout

- `src/lib.rs` — parsed program, limits, isolated execution boundary, pure-data validation, and tests.

## Public API

```rust
pub struct TxProgram { /* parsed immutable form */ }
impl TxProgram {
    pub fn parse(source: &str) -> Result<Self, TxError>;
    pub fn form(&self) -> &cljrs_reader::Form;
}

pub struct TxLimits {
    pub memory_bytes: usize,
    pub gas: u64,
}

pub enum TxError { /* read, boundary, evaluation, budget, and policy errors */ }

pub fn execute(
    program: &TxProgram,
    args: Vec<cljrs_value::clone::SerializedValue>,
    limits: TxLimits,
) -> Result<cljrs_value::clone::SerializedValue, TxError>;
```

`execute` creates and bootstraps a new interpreter environment inside one
bounded arena, structured-clones arguments in, invokes the function under a
gas meter and transaction capability policy, clones a pure-data result out,
and then destroys the whole environment.

The memory limit covers arena object boxes and out-of-line memory reported by
`Trace::gc_size_extra`. It is a managed-allocation limit, not yet a hard RSS
limit: ordinary Rust allocations outside traced values are not fully charged.
A WASM linear-memory or process sandbox remains necessary when an adversarial
hard address-space ceiling is required.
