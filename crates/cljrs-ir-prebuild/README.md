# cljrs-ir-prebuild

Pre-lowers Clojure namespaces to IR and serializes the result to a bundle file
that can be loaded at startup to skip re-lowering already-compiled functions.

**Status:** Phase 10 (IR tiering) support tooling — implemented. Both a
library (consumed by the `cljrs ir-prebuild` subcommand of the main `cljrs`
binary) and a standalone `cljrs-ir-prebuild` binary.

---

## Purpose

`cljrs`'s IR tiers are normally populated lazily: a function is lowered to IR
only once its tree-walked call count crosses the background-lowering
threshold (see `cljrs-eval`'s "Background lowering"). That is fine for a
long-running process, but wastes the warmup on a short-lived or
cold-start-sensitive target — e.g. an embedder built for `wasm32`, where
there is no background lowering worker thread.

`cljrs-ir-prebuild` runs the lowering pipeline ahead of time: it boots a
standard environment, lowers every function arity in the requested namespaces
to IR, and serializes the result (an `IrBundle`) to a file. The bundle is
loaded back at runtime with `cljrs_eval::load_prebuilt_ir`, which matches
bundle entries to the live `ir_arity_id`s assigned when the target functions
are defined and populates the IR cache directly — the functions execute at
Tier 1 (IR interpreter) from their very first call, with no warmup.

---

## File layout

```
src/
  lib.rs   — run_prebuild: boots an env, lowers namespaces to an IrBundle, writes it to disk
  main.rs  — standalone `cljrs-ir-prebuild` binary: Clap CLI wrapper over lib.rs
```

---

## Public API

```rust
pub struct PrebuildStats {
    pub lowered: usize,      // arities successfully lowered
    pub unsupported: usize,  // arities the lowerer could not handle
    pub output: PathBuf,     // where the bundle was written
}

/// Boot a standard environment, lower every function in `namespaces` to IR
/// (non-`clojure.core` namespaces are `require`d from `src_paths` first), and
/// write the serialized bundle to `output`.
pub fn run_prebuild(
    namespaces: &[String],
    output: &PathBuf,
    src_paths: &[PathBuf],
    verbose: bool,
) -> Result<PrebuildStats, String>;
```

Bundle entries are keyed `"ns/name:param_count"` (or `"ns/name:param_count+"`
for a variadic arity) — the same key scheme `cljrs_eval::load_prebuilt_ir`
matches against live function vars when loading a bundle back in.

---

## CLI usage

Via the main `cljrs` binary (see `crates/cljrs`):

```bash
cljrs ir-prebuild build --ns clojure.core -o core.ir.bin
cljrs ir-prebuild dump core.ir.bin
```

Or the standalone binary:

```bash
cljrs-ir-prebuild --ns clojure.core --ns my.app.core -o bundle.bin --src-path src
cljrs-ir-prebuild dump bundle.bin
```

---

## Dependencies

| Crate | Role |
|-------|------|
| `cljrs-types` (workspace) | `Span` |
| `cljrs-reader` (workspace) | `Form`, `FormKind` — builds the synthetic `(require 'ns)` form |
| `cljrs-value` (workspace) | `Value`, `CljxFn` |
| `cljrs-env` (workspace) | `Env`, `GlobalEnv` |
| `cljrs-eval` (workspace) | `standard_env`/`standard_env_with_paths`, `mark_compiler_ready`, `lower::lower_arity` |
| `cljrs-ir` (workspace) | `IrBundle`, `serialize_bundle`/`deserialize_bundle` |
| `clap` (workspace, bin only) | CLI argument parsing |
